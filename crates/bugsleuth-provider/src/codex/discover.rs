//! Finding the Codex CLI on disk.
//!
//! Codex ships as a native executable in more than one place depending on how
//! it was installed, and the npm layout has moved between versions. Each
//! candidate below is a real observed install location; the bare name on PATH
//! is the last resort.

use std::path::PathBuf;

use crate::find::which;

pub(super) fn resolve_binary() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = home {
        let candidates = [
            home.join("AppData/Local/Programs/OpenAI/Codex/bin/codex.exe"),
            home.join(".codex/packages/standalone/current/bin/codex.exe"),
            home.join(".local/bin/codex"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which("codex")
}
