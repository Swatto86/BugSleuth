//! Clone through the user's Git installation and credential helpers.
use crate::cancel::Cancel;
use anyhow::{Context, Result, bail};
use bugsleuth_provider::process::{self, Invocation};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn validate_source(source: &str) -> Result<()> {
    if source.is_empty() || source.starts_with('-') || source.chars().any(char::is_control) {
        bail!("Enter an HTTPS or SSH repository address, or an absolute local path.");
    }
    if let Some(rest) = source.strip_prefix("https://") {
        if rest
            .split('/')
            .next()
            .is_none_or(|host| host.is_empty() || host.contains('@'))
            || rest.contains(['?', '#'])
        {
            bail!(
                "Use an HTTPS address without embedded credentials, query parameters or fragments; authenticate through Git instead."
            );
        }
    } else if let Some(rest) = source.strip_prefix("ssh://") {
        if rest.split('/').next().is_none_or(|host| {
            host.is_empty()
                || host
                    .rsplit_once('@')
                    .is_some_and(|(user, _)| user.contains(':'))
        }) {
            bail!("Use an SSH address without a password.");
        }
    } else if !Path::new(source).is_absolute() {
        let scp = source.split_once('@').is_some_and(|(user, remote)| {
            !user.is_empty()
                && !user.contains([':', '/', ' '])
                && remote.split_once(':').is_some_and(|(host, path)| {
                    !host.is_empty()
                        && !host.contains(['/', '@', ' '])
                        && !path.is_empty()
                        && !path.starts_with(':')
                })
        });
        if !scp {
            bail!("Use HTTPS, SSH (git@host:owner/repo.git), or an absolute local path.");
        }
    }
    Ok(())
}

/// Reserve a new directory; never overwrite or remove an existing destination.
/// Failed or cancelled clones are retained for inspection, never selected.
pub async fn clone_repository(
    source: &str,
    parent: &Path,
    name: &str,
    cancel: &Cancel,
) -> Result<PathBuf> {
    let source = source.trim();
    validate_source(source)?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
    {
        bail!("Choose a new folder name without path separators or special characters.");
    }
    let parent = crate::git_path(
        &tokio::fs::canonicalize(parent)
            .await
            .context("Choose an existing destination parent folder")?,
    );
    if cancel.stopped() {
        bail!("Clone stopped before creating a destination.");
    }
    let destination = parent.join(name);
    tokio::fs::create_dir(&destination)
        .await
        .context("Cannot create destination; choose a folder name that does not already exist")?;
    let args = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
        "-c",
        "protocol.ssh.allow=always",
        "-c",
        "protocol.file.allow=always",
        "clone",
        "--template=",
        "--",
        source,
        name,
    ]
    .map(str::to_string);
    // Preserve authentication/session and proxy integration without passing Git
    // repository overrides or accepting credentials in the URL.
    let mut env: Vec<(String, String)> = [
        "SSH_AUTH_SOCK",
        "SSH_AGENT_PID",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_ASKPASS",
        "GH_CONFIG_DIR",
        "SSH_ASKPASS",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
        "no_proxy",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v)))
    .collect();
    env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
    let result = tokio::select! {
        result = process::run(Invocation { binary: "git", args: &args, cwd: &parent, stdin: None, env: &env, timeout: Duration::from_secs(1800), what: "Git clone" }) => result,
        () = cancel.cancelled() => bail!("Clone stopped. Any partial destination was kept; choose a new folder to retry."),
    };
    // Git/helper diagnostics can contain credentials. Do not forward raw output.
    match result {
        Ok(output) if output.succeeded() => Ok(destination),
        _ => bail!(
            "Git clone failed or timed out. Check Git is installed, the address is correct, and Git can access it using your credential helper or SSH agent. Any partial destination was kept; choose a new folder to retry."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credentials_and_unsafe_transports() {
        for source in [
            "",
            "-u",
            "ext::sh x",
            "https://token@host/repo",
            "https://host/repo?token=x",
            "ssh://user:password@host/repo",
            "http://host/repo",
        ] {
            assert!(validate_source(source).is_err(), "{source}");
        }
        for source in [
            "https://github.com/org/repo.git",
            "git@github.com:org/repo.git",
            "ssh://git@host/repo",
            "ssh://host:2222/repo",
            "ssh://git@host:2222/repo",
        ] {
            assert!(validate_source(source).is_ok(), "{source}");
        }
    }
}
