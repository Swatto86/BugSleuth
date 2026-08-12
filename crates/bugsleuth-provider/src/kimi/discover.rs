//! Finding the Kimi Code CLI on disk.
//!
//! Two installation routes exist and they land in different places. The
//! official installer puts a native executable in `~/.kimi-code/bin` and does
//! **not** necessarily put it on PATH — measured against a real install, where
//! `kimi` was unresolvable as a command while the binary was sitting there. An
//! npm global install instead puts a `kimi.cmd` shim in the npm prefix.
//!
//! So PATH is the last resort rather than the first: a CLI this tool cannot
//! find is reported as "not installed", which is a confusing thing to be told
//! about software you just installed.

use std::path::PathBuf;

use crate::find::which;

pub(super) fn resolve_binary() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    resolve_binary_from(home)
}

/// Look for the installer layout under `home`, then fall back to PATH.
pub(super) fn resolve_binary_from(home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(home) = home {
        let candidates = [
            // The official installer's own layout, native executable first.
            home.join(".kimi-code/bin/kimi.exe"),
            home.join(".kimi-code/bin/kimi"),
            // An npm global install.
            home.join("AppData/Roaming/npm/kimi.cmd"),
            home.join(".local/bin/kimi"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which("kimi")
}
