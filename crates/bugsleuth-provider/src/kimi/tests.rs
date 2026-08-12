//! Tests for the Kimi adapter, in their own file only because the module plus
//! its tests would cross the hard line cap.

use super::*;

fn spec<'a>(model: &'a str, worktree: &'a Path) -> KimiSweep<'a> {
    KimiSweep {
        worktree,
        model,
        brief: "find the defects",
        timeout: Duration::from_secs(60),
        binary: None,
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-kimi-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

/// The selected model reaches the CLI, and an unselected one is not invented.
///
/// This is the whole reason the adapter exists: a subscription reaches models a
/// key-based route does not, and `-m` is what asks for one. Passing an empty
/// `-m` would override the CLI's own `default_model` with nothing.
#[test]
fn the_selected_model_is_passed_and_an_empty_one_is_omitted() {
    let dir = scratch("model-arg");
    let brief =
        brief_file::BriefFile::write("brief", brief_file::REVIEW_AGENT).expect("write brief");

    let chosen = build_args(&spec("kimi-k3", &dir), &brief);
    let at = chosen
        .iter()
        .position(|arg| arg == "-m")
        .expect("a selected model must reach the CLI");
    assert_eq!(chosen.get(at + 1).map(String::as_str), Some("kimi-k3"));

    let default = build_args(&spec("  ", &dir), &brief);
    assert!(
        !default.iter().any(|arg| arg == "-m"),
        "an unselected model overrode the CLI's own default: {default:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The brief crosses as a file, never as an argument.
///
/// `cmd.exe` caps a command line at 8,191 characters and Rust runs the `.cmd`
/// shim through it. A 12 KB brief passed inline is truncated or refused, and
/// neither failure names the cause.
#[test]
fn the_brief_never_reaches_the_command_line() {
    let dir = scratch("brief-size");
    let body = "GIGANTIC".repeat(2_000);
    let brief = brief_file::BriefFile::write(&body, brief_file::REVIEW_AGENT).expect("write brief");
    let args = build_args(
        &KimiSweep {
            brief: &body,
            ..spec("kimi-k3", &dir)
        },
        &brief,
    );

    assert!(
        !args.iter().any(|arg| arg.contains("GIGANTIC")),
        "the brief was passed as an argument"
    );
    let line: usize = args.iter().map(|arg| arg.len() + 3).sum();
    assert!(
        line < 8_191,
        "the command line is {line} characters; cmd.exe refuses it"
    );
    // The known-present half: the argv really does name the brief, so a build
    // that simply dropped it would not satisfy the assertions above.
    assert!(
        args.iter()
            .any(|arg| arg.contains(&brief.path().display().to_string())),
        "the argv does not point at the brief at all: {args:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The brief's directory is granted, or the prompt points at an unreadable file.
#[test]
fn the_brief_directory_is_granted_to_the_session() {
    let dir = scratch("add-dir");
    let brief =
        brief_file::BriefFile::write("brief", brief_file::REVIEW_AGENT).expect("write brief");
    let args = build_args(&spec("", &dir), &brief);
    let at = args
        .iter()
        .position(|arg| arg == "--add-dir")
        .expect("the brief lives outside the worktree and must be granted");
    assert_eq!(
        args.get(at + 1).map(String::as_str),
        Some(brief.dir().to_string_lossy().as_ref()),
        "a directory other than the brief's was granted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A real invocation, against a stub that answers the way Kimi does.
///
/// The parts each have their own test above; this is the one that proves the
/// argv, the working directory, the exit code and the JSON extraction agree
/// with each other at the process boundary.
#[tokio::test]
async fn a_real_invocation_returns_the_findings_the_cli_printed() {
    let dir = scratch("real-invocation");
    let reply = r#"{"findings":[]}"#;

    #[cfg(windows)]
    let stub = {
        let path = dir.join("kimi.cmd");
        std::fs::write(&path, format!("@echo off\r\necho {reply}\r\nexit /b 0\r\n"))
            .expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("kimi.sh");
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{reply}'\n"))
            .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };

    let result = sweep(KimiSweep {
        binary: Some(&stub.to_string_lossy()),
        ..spec("kimi-k3", &dir)
    })
    .await
    .expect("a well-shaped reply is a successful sweep");
    assert!(result.findings.findings.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A CLI that fails is a failure, not an empty review.
///
/// The one outcome this project must never produce is a lane that quietly reads
/// as clean when it never ran.
#[tokio::test]
async fn a_failing_cli_is_an_error_rather_than_no_findings() {
    let dir = scratch("failure");

    #[cfg(windows)]
    let stub = {
        let path = dir.join("kimi.cmd");
        std::fs::write(&path, "@echo off\r\necho boom 1>&2\r\nexit /b 3\r\n").expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("kimi.sh");
        std::fs::write(&path, "#!/bin/sh\necho boom >&2\nexit 3\n").expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };

    let error = sweep(KimiSweep {
        binary: Some(&stub.to_string_lossy()),
        ..spec("", &dir)
    })
    .await
    .expect_err("a non-zero exit is not a clean review");
    let shown = error.to_string();
    assert!(
        shown.contains("boom"),
        "the CLI's own reason was lost: {shown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Discovery finds the CLI where its own installer puts it.
///
/// Measured against the official installer's layout: `~/.kimi-code/bin/kimi`
/// (or `kimi.exe` on Windows). A host-dependent skip left CI green while the
/// candidate list in `discover` could be deleted; a fixture home through
/// `resolve_binary_from` keeps the gate honest without env mutation (forbidden
/// by the workspace `unsafe_code` lint).
#[test]
fn discovery_finds_a_real_installation() {
    let home = std::env::temp_dir().join(format!(
        "bugsleuth-kimi-discover-{}",
        std::process::id()
    ));
    let bin_dir = home.join(".kimi-code/bin");
    std::fs::create_dir_all(&bin_dir).expect("fixture bin dir");
    let exe_name = if cfg!(windows) {
        "kimi.exe"
    } else {
        "kimi"
    };
    let binary = bin_dir.join(exe_name);
    std::fs::write(&binary, b"").expect("fixture binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let found =
        discover::resolve_binary_from(Some(home.clone())).expect("fixture install must be discovered");
    assert_eq!(found, binary);
    let _ = std::fs::remove_dir_all(&home);
}

/// The reply is extracted from the CLI's real output, banner and all.
///
/// Captured verbatim from Kimi Code 0.34.0 answering a real review: a version
/// banner, a bulleted line of its own commentary, the JSON, and a resume hint.
/// A stub emitting bare JSON would have proved nothing about any of that.
#[tokio::test]
async fn the_findings_survive_the_cli_s_own_banner_and_resume_hint() {
    let dir = scratch("real-output-shape");
    let observed = concat!(
        "kimi version 0.34.0\n",
        "\u{2022} Read the brief first.\n",
        "\n",
        "\u{2022} {\"findings\":[{\"title\":\"Divide-by-zero in average()\",",
        "\"severity\":\"high\",\"file\":\"pricing.rs\",\"line\":1,\"snippet\":\"items.len()\",",
        "\"explanation\":\"an empty slice panics\",\"failure_scenario\":\"average(&[])\"}]}\n",
        "\n",
        "To resume this session: kimi -r session_7ea76aa2\n",
    );

    #[cfg(windows)]
    let stub = {
        let path = dir.join("kimi.cmd");
        let script: String = std::iter::once("@echo off\r\n".to_string())
            .chain(observed.lines().map(|line| {
                if line.is_empty() {
                    "echo.\r\n".to_string()
                } else {
                    format!("echo {}\r\n", line.replace('>', "^>"))
                }
            }))
            .chain(std::iter::once("exit /b 0\r\n".to_string()))
            .collect();
        std::fs::write(&path, script).expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("kimi.sh");
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncat <<'KIMIEOF'\n{observed}KIMIEOF\n"),
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };

    let result = sweep(KimiSweep {
        binary: Some(&stub.to_string_lossy()),
        ..spec("kimi-code/k3", &dir)
    })
    .await
    .expect("the JSON is in there, surrounded by the CLI's own prose");
    assert_eq!(result.findings.findings.len(), 1);
    assert_eq!(result.findings.findings[0].file, "pricing.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Neither approval flag may be passed: the CLI refuses both with `--prompt`.
#[test]
fn no_approval_flag_is_passed_because_prompt_mode_refuses_them() {
    let dir = scratch("no-approval-flag");
    let brief =
        brief_file::BriefFile::write("brief", brief_file::REVIEW_AGENT).expect("write brief");
    let args = build_args(&spec("kimi-code/k3", &dir), &brief);
    for refused in ["--yolo", "-y", "--auto"] {
        assert!(
            !args.iter().any(|arg| arg == refused),
            "{refused} is rejected alongside --prompt and would stop every sweep: {args:?}"
        );
    }
    // Known-present control: the flags that *are* required are still there.
    assert!(args.iter().any(|arg| arg == "--output-format"));
    assert!(args.iter().any(|arg| arg == "-p"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Skill auto-discovery is disabled, or the reviewed repository briefs its own
/// reviewer.
///
/// `--skills-dir` loads from the given directory *instead of* the discovered
/// user and project ones, so passing a directory that holds only the brief is
/// what turns discovery off.
#[test]
fn project_skill_discovery_is_disabled() {
    let dir = scratch("skills");
    let brief =
        brief_file::BriefFile::write("brief", brief_file::REVIEW_AGENT).expect("write brief");
    let args = build_args(&spec("", &dir), &brief);
    let at = args
        .iter()
        .position(|arg| arg == "--skills-dir")
        .expect("without this, a skill committed to the reviewed tree is loaded");
    let pointed = args.get(at + 1).map(String::as_str).unwrap_or_default();
    assert_eq!(
        pointed,
        brief.skills_dir().to_string_lossy().as_ref(),
        "skills were pointed somewhere that could contain some"
    );
    assert_eq!(
        std::fs::read_dir(pointed)
            .expect("the skills directory must exist or the flag is ignored")
            .count(),
        0,
        "the directory skill discovery is pointed at is not empty"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every sweep runs under an agent definition that cannot delegate.
///
/// This is the cost boundary, not a nicety. Kimi's `tools` frontmatter is an
/// allowlist and omitting it allows *every* tool — so without this file K3
/// spawned subagents on its own initiative and exhausted a whole billing
/// cycle's quota inside one lane, leaving the other four lanes to fail with
/// HTTP 403 and the run to produce nothing.
#[test]
fn every_sweep_runs_under_an_agent_that_cannot_delegate() {
    let dir = scratch("agent-confinement");
    let brief =
        brief_file::BriefFile::write("brief", brief_file::REVIEW_AGENT).expect("write brief");
    let args = build_args(&spec("kimi-code/k3", &dir), &brief);

    let at = args
        .iter()
        .position(|arg| arg == "--agent-file")
        .expect("without an agent definition Kimi is allowed every tool it has");
    assert_eq!(
        args.get(at + 1).map(String::as_str),
        Some(brief.agent_path().to_string_lossy().as_ref())
    );

    let agent = std::fs::read_to_string(brief.agent_path()).expect("the agent file must exist");
    // An allowlist, because omitting `tools` allows everything.
    let allowed: Vec<&str> = agent
        .lines()
        .skip_while(|line| line.trim() != "tools:")
        .skip(1)
        .take_while(|line| line.trim_start().starts_with("- "))
        .map(|line| line.trim().trim_start_matches("- "))
        .collect();
    assert_eq!(
        allowed,
        ["Read", "Grep", "Glob"],
        "the tool allowlist is not what a read-only review needs"
    );
    // The two that spawn subagents must not be in it, under any spelling.
    for spawner in ["Agent", "AgentSwarm", "Bash", "Skill"] {
        assert!(
            !allowed.contains(&spawner),
            "{spawner} is allowed, so a sweep can fan out or run commands"
        );
        assert!(
            agent.contains(&format!("- {spawner}")),
            "{spawner} is not named in disallowedTools; a bare * matches nothing in Kimi"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
