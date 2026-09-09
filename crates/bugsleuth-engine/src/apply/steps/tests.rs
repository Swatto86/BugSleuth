//! Tests for reading a report's work orders back off disk.

use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-steps-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    dir
}

#[test]
fn defects_run_in_report_order_however_the_directory_lists_them() {
    // Positions are two digits in the name and skip acknowledged findings, so
    // neither directory order nor a dense 1..n sequence can be assumed. Sorting
    // by the parsed number is what makes "resume at defect 4" mean the fourth
    // defect of the report rather than the fourth file the OS happened to hand
    // back.
    let dir = scratch("order");
    for name in ["fix-prompt-10.md", "fix-prompt-02.md", "fix-prompt-07.md"] {
        std::fs::write(dir.join(name), format!("work order {name}")).expect("write prompt");
    }
    // A bundle beside them is not a defect and must not become an extra step:
    // it contains every defect, so running it would redo the whole report.
    std::fs::write(dir.join("fix-prompt.md"), "every defect").expect("write bundle");
    // Neither is a prompt still being written, nor an unrelated file.
    std::fs::write(dir.join("fix-prompt-99.md.writing"), "half").expect("write staged");
    std::fs::write(dir.join("last-report.json"), "{}").expect("write report");

    let steps = load(&dir).expect("load steps");
    assert_eq!(
        steps.iter().map(|s| s.position).collect::<Vec<_>>(),
        vec![2, 7, 10]
    );
    assert_eq!(steps[0].prompt, "work order fix-prompt-02.md");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_report_with_only_a_bundle_still_applies_as_one_step() {
    // Run directories written before per-defect prompts existed have only the
    // bundle. Refusing them would turn an old but perfectly good report into an
    // error; it simply cannot be resumed part-way, which it never could.
    let dir = scratch("bundle");
    std::fs::write(dir.join("fix-prompt.md"), "every defect").expect("write bundle");

    let steps = load(&dir).expect("load steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].position, 0);
    assert_eq!(steps[0].prompt, "every defect");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_with_no_prompt_at_all_is_an_error_rather_than_an_empty_run() {
    // An apply with nothing to do must say so. Returning no steps would run the
    // whole tail — strip, push, tag — over a repository nothing had edited, and
    // report a successful fix run that changed nothing.
    let dir = scratch("empty");
    let error = load(&dir).expect_err("an empty directory has no work orders");
    assert!(error.to_string().contains("Run a review first"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}
