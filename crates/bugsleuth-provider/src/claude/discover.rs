//! Finding the CLI on disk.

use std::path::PathBuf;

/// Locate the CLI, preferring a real executable over an npm shim.
///
/// On Windows the npm `claude.cmd` shim has to be run through `cmd.exe`, which
/// would re-expose every argument to shell parsing — and one of our arguments is
/// a JSON Schema full of quotes and braces. The native `claude.exe` next to it
/// takes argv as an array with no shell in the path at all.
pub(super) fn resolve_binary() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = home {
        let candidates = [
            home.join(".local/bin/claude.exe"),
            home.join("AppData/Roaming/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe"),
            home.join(".local/bin/claude"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which("claude")
}

/// Minimal PATH lookup. A dependency for this would be three lines of value.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &["exe", "cmd"]
    } else {
        &[""]
    };
    for directory in std::env::split_paths(&path) {
        for extension in extensions {
            let candidate = if extension.is_empty() {
                directory.join(name)
            } else {
                directory.join(format!("{name}.{extension}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lookup_returns_none_for_a_command_that_cannot_exist() {
        assert_eq!(which("definitely-not-a-real-command-9c1f"), None);
    }

    #[test]
    fn path_lookup_finds_a_command_that_is_always_present() {
        let known = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(which(known).is_some(), "expected to find `{known}` on PATH");
    }
}
