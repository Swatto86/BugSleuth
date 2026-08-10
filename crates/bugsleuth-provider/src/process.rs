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

mod tree;
use tree::KillTree;

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
    Timeout {
        what: String,
        seconds: u64,
        /// Output written before the cutoff, retained so an adapter can recover
        /// the session rather than restarting paid work from scratch.
        output: CliOutput,
    },
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

impl ProcessError {
    /// Output captured before a timeout killed the child, when there is any.
    pub fn output(&self) -> Option<&CliOutput> {
        match self {
            ProcessError::Timeout { output, .. } => Some(output),
            _ => None,
        }
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
    run_inner(invocation, false).await
}

/// Run a subprocess in an isolated process group where the platform supports
/// it, so dropping this future also terminates helpers it started.
pub async fn run_with_process_group(invocation: Invocation<'_>) -> Result<CliOutput, ProcessError> {
    run_inner(invocation, true).await
}

async fn run_inner(
    invocation: Invocation<'_>,
    isolate_process_group: bool,
) -> Result<CliOutput, ProcessError> {
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
    tree::prepare(&mut command, isolate_process_group);

    let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
        binary: invocation.binary.to_string(),
        source,
    })?;
    // Declared *after* `child`, so it runs first when this function unwinds: a
    // tree can only be walked from a process that is still alive, and dropping
    // `child` kills the one at the top of it.
    let mut tree = KillTree::new(child.id(), isolate_process_group);

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

    // Readers live independently of the completion wait, so a timeout can kill
    // the child, drain what it already wrote, and hand session events to the
    // adapter for recovery. Keeping them inside `timeout` discarded those bytes
    // along with the timed-out future.
    let stdout_task = tokio::spawn(read_capped(stdout));
    let stderr_task = tokio::spawn(read_capped(stderr));
    let combined = async {
        let (status, fed) = tokio::join!(child.wait(), feed);
        (status, fed)
    };

    match tokio::time::timeout(invocation.timeout, combined).await {
        Ok((status, fed)) => {
            let status = status.map_err(|source| ProcessError::Wait {
                what: invocation.what.to_string(),
                source,
            })?;
            // Reaped, so the pid is finished with. Anything still answering to
            // it belongs to whoever the OS gave it to next.
            tree.disarm();
            // A prompt that never arrived is not a sweep. Reported as a failed
            // invocation, which the caller already renders as NOT SWEPT with a
            // reason, rather than as an answer to a question never asked.
            fed.map_err(|source| ProcessError::Wait {
                what: format!("{} (writing the prompt)", invocation.what),
                source,
            })?;
            let (out, err) = tokio::join!(stdout_task, stderr_task);
            let out = out.unwrap_or_default();
            let err = err.unwrap_or_default();
            Ok(CliOutput {
                code: status.code(),
                stdout: String::from_utf8_lossy(&out).into_owned(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
            })
        }
        Err(_) => {
            // The tree first, then the process at the top of it: `start_kill`
            // on its own leaves the CLI and everything the CLI started running.
            tree.fire();
            let _ = child.start_kill();
            // Reap before reading the buffers: process exit closes both pipes,
            // which lets the reader tasks return every byte already written.
            let _ = child.wait().await;
            let (out, err) = tokio::join!(stdout_task, stderr_task);
            Err(ProcessError::Timeout {
                what: invocation.what.to_string(),
                seconds: invocation.timeout.as_secs(),
                output: CliOutput {
                    code: None,
                    stdout: String::from_utf8_lossy(&out.unwrap_or_default()).into_owned(),
                    stderr: String::from_utf8_lossy(&err.unwrap_or_default()).into_owned(),
                },
            })
        }
    }
}

/// Windows' "start this console program without a console window".
///
/// Named rather than written as a bare hex literal because `0x0800_0000` at a
/// call site means nothing to the next reader.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Keep a console CLI from opening a window of its own.
///
/// Every provider here is a console program, and Windows gives a console program
/// launched from a windowed process a brand new console — which appears as a
/// black window on the user's desktop. A run is a dozen sweeps, so without this
/// the desktop app flashes a dozen console windows over whatever the user is
/// doing. Observed during a real run: a `conhost.exe` sitting under the CLI child.
#[cfg(windows)]
fn no_console_window(command: &mut Command) {
    // `tokio::process::Command` carries `creation_flags` itself on Windows, so
    // unlike the `std` variant in `bugsleuth-verify` this needs no extension trait.
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Nothing to do: only Windows conjures a console for a child process.
#[cfg(not(windows))]
fn no_console_window(_command: &mut Command) {}

/// Drain a child stream, retaining at most [`OUTPUT_CAP`] bytes but continuing to
/// read (and discard) beyond it so the child is never blocked by a full pipe.
async fn read_capped<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    stream: Option<R>,
) -> Vec<u8> {
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

/// First `max_chars` characters of `s`, with credential-shaped tokens removed,
/// sliced by character so a multi-byte codepoint straddling the limit cannot panic.
///
/// This is the one place a vendor's raw stdout/stderr is turned into text that
/// gets stored and shown — a failed sweep's "NOT SWEPT" reason is built from it.
/// A CLI can print its own secrets there: Kilo dumped an OAuth refresh and access
/// token into stderr on a credential-store error, and that would have persisted
/// in a report on disk. Redaction happens *before* truncation, so a token cannot
/// leak as a surviving prefix.
pub fn preview(s: &str, max_chars: usize) -> String {
    redact_secrets(s).chars().take(max_chars).collect()
}

/// Replace anything shaped like a JSON Web Token with a placeholder.
///
/// JWTs have a distinctive shape — three runs of base64url characters joined by
/// dots, the first beginning `eyJ` (base64 for `{"`) — so the OAuth access and
/// refresh tokens these CLIs carry can be found and blanked without a regex
/// dependency. It catches the token shape, not every conceivable secret; that is
/// the shape that actually leaked, and erring toward redaction of anything that
/// matches is the safe direction for output that is stored and displayed.
pub fn redact_secrets(text: &str) -> String {
    let is_b64url = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with("eyJ")
            && let Some(len) = jwt_len(rest.as_bytes(), &is_b64url)
        {
            out.push_str("<redacted-credential>");
            rest = &rest[len..];
            continue;
        }
        // Advance exactly one character, keeping UTF-8 boundaries intact.
        let step = rest.chars().next().map_or(1, char::len_utf8);
        out.push_str(&rest[..step]);
        rest = &rest[step..];
    }
    out
}

/// The byte length of a JWT at the start of `bytes`, or `None` if it is not one.
///
/// A JWT is `segment.segment.segment`, each segment one or more base64url bytes.
/// All of it is ASCII, so scanning bytes is safe and cannot split a codepoint.
fn jwt_len(bytes: &[u8], is_b64url: &impl Fn(u8) -> bool) -> Option<usize> {
    let segment = |start: usize| -> usize {
        let mut end = start;
        while end < bytes.len() && is_b64url(bytes[end]) {
            end += 1;
        }
        end
    };
    let first = segment(0);
    if first == 0 || bytes.get(first) != Some(&b'.') {
        return None;
    }
    let second = segment(first + 1);
    if second == first + 1 || bytes.get(second) != Some(&b'.') {
        return None;
    }
    let third = segment(second + 1);
    if third == second + 1 {
        return None;
    }
    Some(third)
}

#[cfg(test)]
mod tests;
