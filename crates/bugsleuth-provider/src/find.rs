//! Locating an executable on PATH.
//!
//! Shared by the vendor adapters. A dependency for this would buy three lines.

use std::path::PathBuf;

/// Minimal PATH lookup. A dependency for this would be three lines of value.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
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
    fn returns_none_for_a_command_that_cannot_exist() {
        assert_eq!(which("definitely-not-a-real-command-9c1f"), None);
    }

    #[test]
    fn finds_a_command_that_is_always_present() {
        let known = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(which(known).is_some(), "expected to find `{known}` on PATH");
    }
}
