//! Finding the Cursor Agent CLI on disk.
//!
//! The command users type is `agent`. On Windows the official installer puts a
//! `.cmd` shim on PATH that launches Node through PowerShell — and Rust runs a
//! `.cmd` through `cmd.exe`, which re-quotes argv and caps the command line at
//! 8,191 characters. Sweeps pass a short pointer rather than the brief itself,
//! but a shim is still the wrong thing to spawn when a real `node.exe` sits
//! next to `index.js` in the install tree.
//!
//! So discovery prefers that pair: spawn `node.exe` with `index.js` as the first
//! argument, argv untouched. PATH `agent` is the last resort.

use std::path::{Path, PathBuf};

use crate::find::which;

/// How to start one Cursor Agent process.
#[derive(Debug, Clone)]
pub(super) struct Launch {
    /// Absolute path to the executable (`node.exe` or `agent`).
    pub binary: PathBuf,
    /// Arguments that must precede every CLI flag (the path to `index.js`, when
    /// we are driving Node directly).
    pub prefix: Vec<String>,
}

/// Locate a runnable Cursor Agent, preferring a direct Node entrypoint.
#[must_use]
pub(super) fn resolve() -> Option<Launch> {
    if let Some(launch) = from_install_tree() {
        return Some(launch);
    }
    which("agent").map(|binary| Launch {
        binary,
        prefix: Vec::new(),
    })
}

/// Where the CLI lives, if it is installed — for the model catalogue.
#[must_use]
pub fn binary_path() -> Option<PathBuf> {
    resolve().map(|launch| launch.binary)
}

fn from_install_tree() -> Option<Launch> {
    for root in install_roots() {
        if let Some(launch) = newest_version_launch(&root) {
            return Some(launch);
        }
        // Older layouts put node.exe next to the launcher rather than under
        // versions/<date>-<hash>/.
        if let Some(launch) = launch_at(&root) {
            return Some(launch);
        }
    }
    None
}

fn install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        roots.push(local.join("Programs").join("CursorAgent"));
        roots.push(local.join("cursor-agent"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
    {
        roots.push(home.join(".local").join("share").join("cursor-agent"));
        roots.push(home.join(".cursor-agent"));
    }
    roots
}

fn newest_version_launch(root: &Path) -> Option<Launch> {
    let versions = root.join("versions");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&versions)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Version dirs are `YYYY.MM.DD-<hash>`; lexicographic order matches date order.
    entries.sort();
    entries.into_iter().rev().find_map(|dir| launch_at(&dir))
}

fn launch_at(dir: &Path) -> Option<Launch> {
    let node = {
        let windows = dir.join("node.exe");
        let unix = dir.join("node");
        if windows.is_file() {
            windows
        } else if unix.is_file() {
            unix
        } else {
            return None;
        }
    };
    let index = dir.join("index.js");
    if !index.is_file() {
        return None;
    }
    Some(Launch {
        binary: node,
        prefix: vec![index.to_string_lossy().into_owned()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_at_requires_both_node_and_index() {
        let dir = std::env::temp_dir().join(format!(
            "bugsleuth-cursor-launch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        assert!(launch_at(&dir).is_none());

        let node_name = if cfg!(windows) { "node.exe" } else { "node" };
        std::fs::write(dir.join(node_name), b"x").expect("node");
        assert!(launch_at(&dir).is_none());

        std::fs::write(dir.join("index.js"), b"x").expect("index");
        let launch = launch_at(&dir).expect("both present");
        assert_eq!(launch.binary.file_name().unwrap(), node_name);
        assert_eq!(launch.prefix.len(), 1);
        assert!(launch.prefix[0].ends_with("index.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
