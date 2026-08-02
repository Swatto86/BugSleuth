//! Spawning, feeding, bounding and killing a CLI subprocess.
//!
//! Shared by every adapter. The per-CLI files decide *what* to run; this file
//! owns the parts that are identical and easy to get subtly wrong: not
//! deadlocking on pipes, not buffering an unbounded amount of output, and not
//! leaving an orphan process behind when a sweep is cancelled or times out.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// Per-stream memory ceiling. An agentic CLI on a large repo can emit a lot of
/// stream-json; past this we keep reading (so the child never blocks on a full
/// pipe) but stop retaining.
const OUTPUT_CAP: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("failed to start `{binary}`: {source}")]
    Spawn {
        binary: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{what}` timed out after {seconds}s")]
    Timeout { what: String, seconds: u64 },
    #[error("`{what}` process error: {source}")]
    Wait {
        what: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct CliOutput {
    /// Exit code, or `None` if the process was terminated by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

/// Everything needed to run one CLI invocation.
pub struct Invocation<'a> {
    /// Absolute path to the executable. Adapters resolve a real `.exe` where one
    /// exists rather than an npm `.cmd` shim: a shim must be run through
    /// `cmd.exe`, which reintroduces shell quoting on arguments that contain
    /// JSON. Spawning the binary directly passes argv as an array, untouched.
    pub binary: &'a str,
    pub args: &'a [String],
    /// Working directory — for a sweep this is the repository under review.
    pub cwd: &'a Path,
    /// Written to the child's stdin, which is then closed to signal EOF.
    pub stdin: Option<&'a [u8]>,
    /// Extra environment variables. Used for the API-key path, where setting
    /// `ANTHROPIC_API_KEY` switches the CLI off subscription auth entirely.
    pub env: &'a [(String, String)],
    pub timeout: Duration,
    /// Label used in error messages, e.g. "claude CLI".
    pub what: &'a str,
}

/// Run a CLI to completion, capturing stdout and stderr.
///
/// Non-zero exit is *not* an error here — the caller decides, because some CLIs
/// report a usable result alongside a non-zero code.
pub async fn run(invocation: Invocation<'_>) -> Result<CliOutput, ProcessError> {
    let mut command = Command::new(invocation.binary);
    command
        .args(invocation.args)
        .current_dir(invocation.cwd)
        // Without this a cancelled sweep leaves the CLI running against the
        // worktree we are about to delete.
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in invocation.env {
        command.env(key, value);
    }
    no_console_window(&mut command);

    let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
        binary: invocation.binary.to_string(),
        source,
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    let payload = invocation.stdin.map(<[u8]>::to_vec);

    // stdin must be written CONCURRENTLY with draining stdout/stderr. Writing the
    // whole prompt first deadlocks once the prompt exceeds the OS pipe buffer
    // (~64 KB) and the child starts emitting output before it has consumed all of
    // stdin: the child blocks writing to a full stdout pipe while we block
    // writing to a full stdin pipe. Neither side can move.
    // Whether the prompt actually reached the child. A failed write used to be
    // discarded, so a truncated or empty prompt looked exactly like a normal
    // run: the model answered whatever it had received — possibly nothing — and
    // the report presented that as a sweep. Better to fail loudly than to
    // report a review of a prompt that was never delivered.
    let feed = async move {
        let (Some(mut stdin), Some(bytes)) = (stdin, payload) else {
            return Ok(());
        };
        let written = stdin.write_all(&bytes).await;
        // Dropping `stdin` closes it, which the child sees as EOF.
        written
    };

    let combined = async {
        let (out, err, status, fed) =
            tokio::join!(read_capped(stdout), read_capped(stderr), child.wait(), feed,);
        (out, err, status, fed)
    };

    match tokio::time::timeout(invocation.timeout, combined).await {
        Ok((out, err, status, fed)) => {
            let status = status.map_err(|source| ProcessError::Wait {
                what: invocation.what.to_string(),
                source,
            })?;
            // A prompt that never arrived is not a sweep. Reported as a failed
            // invocation, which the caller already renders as NOT SWEPT with a
            // reason, rather than as an answer to a question never asked.
            fed.map_err(|source| ProcessError::Wait {
                what: format!("{} (writing the prompt)", invocation.what),
                source,
            })?;
            Ok(CliOutput {
                code: status.code(),
                stdout: String::from_utf8_lossy(&out).into_owned(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            })
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(ProcessError::Timeout {
                what: invocation.what.to_string(),
                seconds: invocation.timeout.as_secs(),
            })
        }
    }
}

/// Keep a console CLI from opening a window of its own.
///
/// Every provider here is a console program, and Windows gives a console program
/// launched from a windowed process a brand new console — which appears as a
/// black window on the user's desktop. A run is a dozen sweeps, so without this
/// the desktop app flashes a dozen console windows over whatever the user is
/// doing. Observed during a real run: a `conhost.exe` sitting under the CLI child.
///
/// `CREATE_NO_WINDOW`. Named rather than written as a bare hex literal because
/// `0x0800_0000` at a call site means nothing to the next reader.
#[cfg(windows)]
fn no_console_window(command: &mut Command) {
    // `tokio::process::Command` carries `creation_flags` itself on Windows, so
    // unlike the `std` variant in `bugsleuth-verify` this needs no extension trait.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Nothing to do: only Windows conjures a console for a child process.
#[cfg(not(windows))]
fn no_console_window(_command: &mut Command) {}

/// Drain a child stream, retaining at most [`OUTPUT_CAP`] bytes but continuing to
/// read (and discard) beyond it so the child is never blocked by a full pipe.
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(stream: Option<R>) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(mut reader) = stream else {
        return out;
    };
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < OUTPUT_CAP {
                    let take = n.min(OUTPUT_CAP - out.len());
                    out.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
    out
}

/// First `max_chars` characters of `s`, sliced by character so a multi-byte
/// codepoint straddling the limit cannot panic.
pub fn preview(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("cmd.exe", vec!["/C".into()])
        } else {
            ("/bin/sh", vec!["-c".into()])
        }
    }

    #[tokio::test]
    async fn captures_stdout_and_a_zero_exit_code() {
        let (binary, mut args) = shell();
        args.push("echo hello".into());
        let output = run(Invocation {
            binary,
            args: &args,
            cwd: Path::new("."),
            stdin: None,
            env: &[],
            timeout: Duration::from_secs(30),
            what: "test",
        })
        .await;
        let output = output.unwrap_or_else(|e| panic!("spawn failed: {e}"));
        assert!(output.succeeded(), "expected success, got {output:?}");
        assert!(
            output.stdout.contains("hello"),
            "stdout was {:?}",
            output.stdout
        );
    }

    #[tokio::test]
    async fn reports_a_nonzero_exit_code_rather_than_erroring() {
        let (binary, mut args) = shell();
        args.push("exit 3".into());
        let output = run(Invocation {
            binary,
            args: &args,
            cwd: Path::new("."),
            stdin: None,
            env: &[],
            timeout: Duration::from_secs(30),
            what: "test",
        })
        .await;
        let output = output.unwrap_or_else(|e| panic!("spawn failed: {e}"));
        assert_eq!(output.code, Some(3));
        assert!(!output.succeeded());
    }

    #[tokio::test]
    async fn kills_a_child_that_outlives_its_timeout() {
        let (binary, mut args) = shell();
        // Both shells understand a long-running ping loopback wait on Windows;
        // sleep elsewhere.
        args.push(if cfg!(windows) {
            "ping -n 30 127.0.0.1 >NUL".into()
        } else {
            "sleep 30".to_string()
        });
        let result = run(Invocation {
            binary,
            args: &args,
            cwd: Path::new("."),
            stdin: None,
            env: &[],
            timeout: Duration::from_millis(400),
            what: "slow test",
        })
        .await;
        assert!(
            matches!(result, Err(ProcessError::Timeout { .. })),
            "expected a timeout, got {result:?}"
        );
    }
}
