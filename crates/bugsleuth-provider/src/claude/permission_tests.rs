//! Production-path checks for Claude's review permission boundary.

use super::*;

fn value_after<'a>(argv: &'a str, flag: &str) -> Option<&'a str> {
    argv.split_whitespace()
        .skip_while(|part| *part != flag)
        .nth(1)
        .map(|value| value.trim_matches('"'))
}

#[tokio::test]
async fn read_only_tools_are_repo_scoped() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-claude-read-policy-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create CLI fixture directory");

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("claude.cmd"),
        "@echo off\r\n\
         echo %* > argv.txt\r\n\
         echo {\"result\":\"\",\"structured_output\":{\"findings\":[]},\"is_error\":false,\"session_id\":\"known-session\"}\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("claude"),
        "#!/bin/sh\n\
         printf '%s\\n' \"$*\" > argv.txt\n\
         printf '%s\\n' '{\"result\":\"\",\"structured_output\":{\"findings\":[]},\"is_error\":false,\"session_id\":\"known-session\"}'\n",
    );
    std::fs::write(&stub, script).expect("write CLI stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&stub)
            .expect("read stub permissions")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&stub, permissions).expect("make CLI stub executable");
    }

    let binary = stub.to_string_lossy().into_owned();
    sweep(ClaudeSweep {
        repo: &dir,
        lane: Lane::Security,
        model: "",
        effort: "",
        use_agents: false,
        brief: "",
        timeout: Duration::from_secs(10),
        max_turns: 1,
        binary: Some(&binary),
        api_key: None,
    })
    .await
    .expect("fixture review succeeds");

    let argv = std::fs::read_to_string(dir.join("argv.txt")).expect("captured argv");
    assert_eq!(value_after(&argv, "--tools"), Some("Read,Glob,Grep"));
    let allowed = value_after(&argv, "--allowedTools").expect("approved tool rules");
    assert_eq!(allowed, "Read(./**),Glob(./**)");
    assert!(!allowed.split(',').any(|rule| rule == "Read"));
    assert_eq!(value_after(&argv, "--permission-mode"), Some("dontAsk"));

    let _ = std::fs::remove_dir_all(dir);
}

/// The apply path is the one invocation allowed to write, so what it is granted
/// is checked on the production path rather than against the constant alone.
#[tokio::test]
async fn apply_may_write_but_only_inside_the_repository() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-claude-apply-policy-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create CLI fixture directory");

    #[cfg(windows)]
    let (stub, script) = (
        dir.join("claude.cmd"),
        "@echo off\r\n\
         echo %* > argv.txt\r\n\
         echo {\"result\":\"fixed it\",\"is_error\":false,\"session_id\":\"known-session\"}\r\n",
    );
    #[cfg(unix)]
    let (stub, script) = (
        dir.join("claude"),
        "#!/bin/sh\n\
         printf '%s\\n' \"$*\" > argv.txt\n\
         printf '%s\\n' '{\"result\":\"fixed it\",\"is_error\":false,\"session_id\":\"known-session\"}'\n",
    );
    std::fs::write(&stub, script).expect("write CLI stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&stub)
            .expect("read stub permissions")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&stub, permissions).expect("make CLI stub executable");
    }

    let binary = stub.to_string_lossy().into_owned();
    let report = apply(ApplyRequest {
        repo: &dir,
        model: "",
        effort: "",
        prompt: "fix the finding",
        timeout: Duration::from_secs(10),
        max_turns: 1,
        binary: Some(&binary),
    })
    .await
    .expect("fixture apply succeeds");
    assert_eq!(
        report, "fixed it",
        "the model's own account was not returned"
    );

    let argv = std::fs::read_to_string(dir.join("argv.txt")).expect("captured argv");
    let tools = value_after(&argv, "--tools").expect("available tools");
    for tool in ["Edit", "Write", "Bash"] {
        assert!(
            tools.split(',').any(|available| available == tool),
            "applying cannot {tool}: {tools}"
        );
    }
    let allowed = value_after(&argv, "--allowedTools").expect("approved tool rules");
    assert_eq!(
        allowed,
        "Read(./**),Glob(./**),Grep,Edit(./**),Write(./**),Bash"
    );
    for unscoped in ["Edit", "Write"] {
        assert!(
            !allowed.split(',').any(|rule| rule == unscoped),
            "{unscoped} is granted over the whole filesystem: {allowed}"
        );
    }
    assert_eq!(
        value_after(&argv, "--disallowedTools"),
        Some("WebFetch,WebSearch")
    );

    let _ = std::fs::remove_dir_all(dir);
}
