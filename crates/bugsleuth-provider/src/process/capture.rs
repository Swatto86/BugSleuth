//! Bounded draining of one child-process stream.

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{CliOutput, ProcessError};

#[derive(Debug, Default)]
pub(super) struct Captured {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

/// Keep at most `cap` bytes, but drain to EOF so a full pipe cannot block the
/// child. The first byte beyond the cap marks the retained prefix incomplete;
/// exactly `cap` bytes followed by EOF is still complete.
pub(super) async fn read_into<R: AsyncRead + Unpin>(
    stream: Option<R>,
    cap: usize,
    captured: &mut Captured,
) -> std::io::Result<()> {
    let Some(mut reader) = stream else {
        return Ok(());
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => return Ok(()),
            Ok(n) => {
                let take = n.min(cap.saturating_sub(captured.bytes.len()));
                captured.bytes.extend_from_slice(&chunk[..take]);
                captured.truncated |= take < n;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn check_reads(
    what: &str,
    stdout: std::io::Result<()>,
    stderr: std::io::Result<()>,
) -> Result<(), ProcessError> {
    stdout.map_err(|source| ProcessError::Read {
        what: what.to_string(),
        stream: "stdout",
        source,
    })?;
    stderr.map_err(|source| ProcessError::Read {
        what: what.to_string(),
        stream: "stderr",
        source,
    })
}

pub(super) fn result(
    what: &str,
    limit: usize,
    code: Option<i32>,
    stdout: Captured,
    stderr: Captured,
    timeout_seconds: Option<u64>,
) -> Result<CliOutput, ProcessError> {
    let stdout_truncated = stdout.truncated;
    let stderr_truncated = stderr.truncated;
    let output = CliOutput {
        code,
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
    };
    if let Some(seconds) = timeout_seconds {
        return Err(ProcessError::Timeout {
            what: what.to_string(),
            seconds,
            output,
        });
    }
    if stdout_truncated || stderr_truncated {
        let streams = if stdout_truncated && stderr_truncated {
            "stdout and stderr"
        } else if stdout_truncated {
            "stdout"
        } else {
            "stderr"
        };
        let context = timeout_seconds.map_or_else(
            || "when the process exited".to_string(),
            |seconds| format!("when it timed out after {seconds}s"),
        );
        return Err(ProcessError::OutputTruncated {
            what: what.to_string(),
            streams,
            limit,
            context,
            output: Box::new(output),
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, ReadBuf};

    use super::*;

    struct BytesThenError(bool);

    impl AsyncRead for BytesThenError {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.0 {
                Poll::Ready(Err(std::io::Error::other("pipe failed")))
            } else {
                self.0 = true;
                buf.put_slice(b"partial");
                Poll::Ready(Ok(()))
            }
        }
    }

    #[tokio::test]
    async fn a_pipe_read_error_is_not_eof() {
        let mut captured = Captured::default();
        let read = read_into(Some(BytesThenError(false)), 1024, &mut captured).await;
        let error = check_reads("test provider", read, Ok(()))
            .expect_err("a failed stdout read was reported as clean EOF");

        assert_eq!(captured.bytes, b"partial");
        match error {
            ProcessError::Read {
                what,
                stream,
                source,
            } => {
                assert_eq!(what, "test provider");
                assert_eq!(stream, "stdout");
                assert_eq!(source.to_string(), "pipe failed");
            }
            other => panic!("wrong capture error: {other}"),
        }
    }
}
