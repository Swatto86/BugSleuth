//! Descendants of an ordinary `run()` must die with the invocation.
//!
//! Windows already covers tree kill via `taskkill /T` in `tests.rs`. On Unix the
//! same guarantee needs an isolated process group; without it, `KillTree::fire`
//! is a no-op and only the direct child dies. The source scan below is the
//! cross-platform gate that ordinary `run` enables that isolation; the Unix
//! subprocess test is the observed effect.

/// Ordinary `run` must isolate a process group, same as `run_with_process_group`.
///
/// On Unix that flag is what makes `KillTree` signal the whole group. Leaving it
/// false is a silent no-op for descendants — the defect this module exists for.
#[test]
fn ordinary_run_isolates_the_process_group() {
    let source = include_str!("../process.rs");
    let code = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(before, _)| before);
    let run_fn = code
        .split("pub async fn run(invocation: Invocation<'_>)")
        .nth(1)
        .and_then(|rest| rest.split("pub async fn run_with_process_group").next())
        .expect("run() definition");
    assert!(
        run_fn.contains("run_inner(invocation, true, OUTPUT_CAP)"),
        "ordinary run() must isolate process groups so Unix descendants die on \
         cancel/timeout; got: {run_fn}"
    );
    assert!(
        !run_fn.contains("run_inner(invocation, false, OUTPUT_CAP)"),
        "ordinary run() still disables process-group isolation"
    );
}

/// Ordinary `run()` must kill helpers the child started, not only the child.
///
/// A shell backgrounds a writer that would leave a late marker, then sleeps.
/// Timing out the invocation must reap that helper — otherwise a cancelled
/// provider CLI leaves quota-consuming descendants behind.
#[cfg(unix)]
#[tokio::test]
async fn a_timeout_on_run_kills_unix_descendants_not_only_the_child() {
    use super::*;
    use std::time::Duration;

    let dir = scratch("unix-tree-kill");
    let started = dir.join("started.txt");
    let survived = dir.join("survived.txt");
    let started_q = started.to_string_lossy().replace('\'', "'\\''");
    let survived_q = survived.to_string_lossy().replace('\'', "'\\''");
    // Background job stays in the shell's process group under a non-interactive
    // `/bin/sh -c` (no job control). Isolating that group is what lets one
    // signal reach the helper; without it, killing the shell leaves it alive.
    let script = format!("touch '{started_q}'; (sleep 5; touch '{survived_q}') & sleep 30");
    let args = vec!["-c".into(), script];

    let result = run(Invocation {
        binary: "/bin/sh",
        args: &args,
        cwd: &dir,
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(1),
        what: "unix tree test",
    })
    .await;
    assert!(
        matches!(result, Err(ProcessError::Timeout { .. })),
        "expected a timeout, got {result:?}"
    );

    // Wait past when the helper would have written its marker if still alive.
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(
        started.exists(),
        "the child never ran, so this proves nothing about killing descendants"
    );
    assert!(
        !survived.exists(),
        "a descendant outlived ordinary run() after timeout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-process-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}
