//! Finding the Kilo CLI on disk.
//!
//! Kilo installs via npm and ships no native executable, so unlike the other two
//! vendors there is no `.exe` to prefer. The `.cmd` shim is what exists, and it
//! is run through the process spawner directly — safe here only because this
//! adapter passes no argument containing shell metacharacters (no inline JSON
//! schema, unlike Claude).

use std::path::PathBuf;

use crate::find::which;

pub(super) fn resolve_binary() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if let Some(home) = home {
        let candidates = [
            home.join("AppData/Roaming/npm/kilo.cmd"),
            home.join(".local/bin/kilo"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which("kilo")
}
