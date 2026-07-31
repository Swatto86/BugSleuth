//! Finding the CLI on disk.

use std::path::PathBuf;

use crate::find::which;

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
