//! A reviewed repository's `core.fsmonitor` must not run during apply git.

use super::*;
use crate::apply::Baseline;

/// `changed_since` runs `git status` / `ls-files` here, and git honours
/// `core.fsmonitor` — a local config entry pointing at an executable of the
/// repository's choosing. The `-c core.fsmonitor=false` override ahead of the
/// subcommand stops it executing that code with the user's permissions.
#[test]
fn a_repo_fsmonitor_hook_is_not_run_during_apply() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-apply-fsmonitor-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
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

    // Unborn baseline still hits `git status` / `ls-files` through `git_with_env`.
    let _ = changed_since(&dir, &Baseline::Unborn);
    assert!(
        !marker.exists(),
        "the repository's fsmonitor hook was executed during apply git"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
