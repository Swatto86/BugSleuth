//! Pinning a sweep to the exact source revision it reviewed.
//!
//! Split from `sweep.rs` at the hard line cap, along a real seam: that file runs
//! the review, and this answers the two git questions around it — what commit
//! was reviewed, and whether the tree is clean enough to reuse a sweep later.
//!
//! Every git call here goes through one window-hiding helper. On the desktop a
//! console program spawned from the windowed app is given a brand-new console —
//! a black window flashing over the user's desktop — and a sweep asks git twice,
//! so two more of those per run leaked through until these were routed together.

use std::path::Path;

/// Run a read-only git query with no console window, returning its output.
///
/// Both callers below spawn `git` on the desktop, where a console program
/// launched from the windowed app gets a brand-new console window. Routed
/// through one place so the window-hiding cannot be forgotten by one and
/// remembered by the other — the same guard `observed.rs` and `gaps.rs` use.
///
/// A reviewed repository is untrusted, and git reads its `.git/config` on every
/// invocation: `core.fsmonitor` would point git at an executable of the
/// repository's choosing, and a hooks path can be set the same way. Both are
/// disabled ahead of the subcommand, so the preflight and revision probes
/// cannot execute repository-controlled code.
fn git_query(repo: &Path, args: &[&str]) -> Option<std::process::Output> {
    bugsleuth_verify::hide_console_window(&mut std::process::Command::new("git"))
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .ok()
}

/// The commit HEAD points at, or nothing for a directory that is not a git
/// repository. Never an error: provenance is worth recording, not worth
/// failing a sweep over.
pub(super) fn reviewed_commit(repo: &Path) -> Option<String> {
    let output = git_query(repo, &["rev-parse", "HEAD"])?;
    if !output.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!hash.is_empty()).then_some(hash)
}

/// The commit HEAD points at, but only when the working tree is clean.
///
/// A sweep is reusable only if it can be pinned to an exact source revision.
/// A dirty tree cannot be — its content is not any commit — so this returns
/// `None` for a dirty or non-git directory, and only a clean checkout yields a
/// revision a later run can compare against.
pub(crate) fn clean_revision(repo: &Path) -> Option<String> {
    let output = git_query(repo, &["status", "--porcelain", "--untracked-files=all"])?;
    if !output.status.success() || !output.stdout.is_empty() {
        return None;
    }
    reviewed_commit(repo)
}

#[cfg(test)]
mod tests {
    /// Every git spawn here hides its console window.
    ///
    /// On Windows a console program spawned from the windowed desktop app is
    /// given a new console — a black window over the user's desktop. This was
    /// reported from a real run. A source scan, in the codebase's own style for
    /// cross-cutting spawn invariants: git is spawned in exactly one place, and
    /// that place hides the window. A new bare `Command::new("git")` here trips
    /// the count; dropping the guard trips the second assertion.
    #[test]
    fn git_is_never_spawned_with_a_visible_console_window() {
        let source = include_str!("revision.rs");
        let code = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(before, _)| before);
        assert_eq!(
            code.matches("Command::new(\"git\")").count(),
            1,
            "git should be spawned in exactly one guarded place"
        );
        assert!(
            code.contains("hide_console_window(&mut std::process::Command::new(\"git\"))"),
            "the git spawn does not hide its console window"
        );
    }

    /// A reviewed repository's `core.fsmonitor` must not run during the
    /// revision probe. `clean_revision` runs `git status` here, and git honours
    /// `core.fsmonitor` — a local config entry pointing at an executable of the
    /// repository's choosing. The `-c core.fsmonitor=false` override ahead of
    /// the subcommand stops it executing that code with the user's permissions.
    #[test]
    fn a_repository_fsmonitor_hook_is_not_run_by_the_revision_probe() {
        let dir = std::env::temp_dir().join(format!("bugsleuth-fsmonitor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&dir)
                    .output()
                    .expect("git")
                    .status
                    .success(),
                "git {args:?} failed"
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@x.invalid"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "x\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "base"]);

        // A hook that leaves a marker behind if git ever runs it.
        let marker = dir.join("fsmonitor_ran");
        #[cfg(windows)]
        {
            let hook = dir.join("fsmonitor-hook.cmd");
            std::fs::write(
                &hook,
                format!("@echo off\r\necho ran > \"{}\"\r\n", marker.display()),
            )
            .expect("hook");
            git(&[
                "config",
                "core.fsmonitor",
                &hook.to_string_lossy().replace('\\', "/"),
            ]);
        }
        #[cfg(unix)]
        {
            let hook = dir.join("fsmonitor-hook");
            std::fs::write(
                &hook,
                format!("#!/bin/sh\necho ran > \"{}\"\n", marker.display()),
            )
            .expect("hook");
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
            git(&["config", "core.fsmonitor", &hook.to_string_lossy()]);
        }

        let _ = super::clean_revision(&dir);
        assert!(
            !marker.exists(),
            "the repository's fsmonitor hook was executed during the revision probe"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
