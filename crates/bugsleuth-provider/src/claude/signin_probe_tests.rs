//! Sign-in probe isolation for the Claude adapter.

use super::super::claude::*;

/// The sign-in probe runs before any sweep, so it must use the same
/// customization boundary a real review does and must not run from the caller's
/// working directory. A stub CLI records the argv and cwd it was handed; the
/// probe is asserted to carry `--safe-mode` and to have run from somewhere
/// other than the process's own directory.
#[tokio::test]
async fn the_signin_probe_uses_safe_mode_and_a_private_directory() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-claude-signin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("claude.cmd"),
        format!(
            "@echo off\r\necho %* > \"{dir}\\args.txt\"\r\necho %CD% > \"{dir}\\cwd.txt\"\r\n",
            dir = dir.display()
        ),
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("claude"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"{dir}/args.txt\"\npwd > \"{dir}/cwd.txt\"\n",
            dir = dir.display()
        ),
    );
    std::fs::write(&stub, script).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();
    }

    let _ = signin(Some(&stub.to_string_lossy())).await;

    let args = std::fs::read_to_string(dir.join("args.txt")).expect("argv captured");
    assert!(
        args.contains("--safe-mode"),
        "the sign-in probe did not disable customizations: {args}"
    );
    let probe_cwd = std::fs::read_to_string(dir.join("cwd.txt"))
        .expect("cwd captured")
        .trim()
        .to_string();
    let here = std::env::current_dir().expect("process cwd");
    let probe_canonical = std::fs::canonicalize(&probe_cwd).unwrap_or_default();
    let here_canonical = here.canonicalize().unwrap_or_default();
    assert_ne!(
        probe_canonical, here_canonical,
        "the sign-in probe ran from the caller's directory rather than a private one: {probe_cwd}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
