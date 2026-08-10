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
) {
    let Some(mut reader) = stream else {
        return;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let take = n.min(cap.saturating_sub(captured.bytes.len()));
                captured.bytes.extend_from_slice(&chunk[..take]);
                captured.truncated |= take < n;
            }
        }
    }
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
    match timeout_seconds {
        Some(seconds) => Err(ProcessError::Timeout {
            what: what.to_string(),
            seconds,
            output,
        }),
        None => Ok(output),
    }
}
