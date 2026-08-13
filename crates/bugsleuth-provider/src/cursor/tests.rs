//! Tests for the Cursor adapter.

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-cursor-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

fn spec<'a>(model: &'a str, worktree: &'a Path) -> CursorSweep<'a> {
    CursorSweep {
        worktree,
        model,
        brief: "find the defects",
        timeout: Duration::from_secs(60),
        binary: None,
    }
}

#[test]
fn the_selected_model_is_passed_and_an_empty_one_is_omitted() {
    let dir = scratch("model-arg");
    let brief = brief_file::BriefFile::write_in(&dir, "brief").expect("write brief");

    let chosen = build_args(&[], &spec("composer-2.5", &dir), &brief);
    let at = chosen
        .iter()
        .position(|arg| arg == "--model")
        .expect("a selected model must reach the CLI");
    assert_eq!(chosen.get(at + 1).map(String::as_str), Some("composer-2.5"));

    let default = build_args(&[], &spec("  ", &dir), &brief);
    assert!(
        !default.iter().any(|arg| arg == "--model"),
        "an unselected model overrode the CLI's own default: {default:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sweep_is_read_only_ask_mode_and_never_forces_writes() {
    let dir = scratch("ask-mode");
    let brief = brief_file::BriefFile::write_in(&dir, "brief").expect("write brief");
    let args = build_args(&[], &spec("auto", &dir), &brief);

    let mode = args.iter().position(|a| a == "--mode").expect("--mode");
    assert_eq!(args.get(mode + 1).map(String::as_str), Some("ask"));
    assert!(args.iter().any(|a| a == "-p"));
    assert!(args.iter().any(|a| a == "--trust"));
    assert!(!args.iter().any(|a| a == "--force" || a == "--yolo"));

    let ws = args
        .iter()
        .position(|a| a == "--workspace")
        .expect("workspace");
    assert_eq!(
        args.get(ws + 1).map(String::as_str),
        Some(dir.to_string_lossy().as_ref())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_brief_never_reaches_the_command_line() {
    let dir = scratch("brief-size");
    let body = "GIGANTIC".repeat(2_000);
    let brief = brief_file::BriefFile::write_in(&dir, &body).expect("write brief");
    let args = build_args(
        &[],
        &CursorSweep {
            brief: &body,
            ..spec("composer-2.5", &dir)
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
    assert!(
        args.iter().any(|arg| {
            brief
                .path()
                .file_name()
                .is_some_and(|name| arg.contains(&*name.to_string_lossy()))
        }),
        "the argv must name the brief file: {args:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn node_prefix_args_come_before_the_cli_flags() {
    let dir = scratch("prefix");
    let brief = brief_file::BriefFile::write_in(&dir, "brief").expect("write brief");
    let args = build_args(&["C:/agent/index.js".into()], &spec("auto", &dir), &brief);
    assert_eq!(args.first().map(String::as_str), Some("C:/agent/index.js"));
    assert_eq!(args.get(1).map(String::as_str), Some("-p"));
    let _ = std::fs::remove_dir_all(&dir);
}
