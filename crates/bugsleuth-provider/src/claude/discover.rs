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

    resolve_from(which("claude"), home)
}

fn resolve_from(path: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    // Respect the native executable selected by PATH on Unix. A fallback
    // wrapper can print setup banners (or select a different CLI version).
    if cfg!(unix) && path.is_some() {
        return path;
    }
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
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_selection_and_native_windows_preference_are_preserved() {
        let dir =
            std::env::temp_dir().join(format!("bugsleuth-claude-discovery-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".local/bin")).unwrap();
        let native = dir.join(".local/bin/claude.exe");
        std::fs::write(&native, "native fixture").unwrap();
        let selected = dir.join("selected-on-path");
        std::fs::write(&selected, "path fixture").unwrap();
        let actual = resolve_from(Some(selected.clone()), Some(dir.clone()));
        assert_eq!(
            actual,
            Some(if cfg!(unix) { selected } else { native.clone() })
        );
        assert_eq!(resolve_from(None, Some(dir.clone())), Some(native));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
