//! Cancellable network Git commands used by push and release publication.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use crate::cancel::Cancel;

#[derive(Debug)]
pub(super) enum Error {
    Cancelled,
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("publication was cancelled"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

pub(super) async fn git(
    repo: &Path,
    args: &[&str],
    cancel: &Cancel,
    timeout: Duration,
) -> Result<String, Error> {
    let args = args
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    let operation = format!("git {}", args.first().map_or("?", String::as_str));
    let running = bugsleuth_provider::process::run_with_process_group(
        bugsleuth_provider::process::Invocation {
            binary: "git",
            args: &args,
            cwd: repo,
            stdin: None,
            env: &[],
            timeout,
            what: &operation,
        },
    );
    tokio::pin!(running);
    let output = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(Error::Cancelled),
        output = &mut running => output.map_err(|error| Error::Failed(error.to_string()))?,
    };
    if !output.succeeded() {
        return Err(Error::Failed(format!(
            "{operation} failed: {}",
            bugsleuth_provider::process::preview(&output.stderr, 4096).trim()
        )));
    }
    Ok(output.stdout)
}
