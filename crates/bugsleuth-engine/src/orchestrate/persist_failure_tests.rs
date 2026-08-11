//! A completed sweep that could not be saved must fail the run.

use super::super::orchestrate::*;
use super::super::plan::{Config, ModelPlan, plan};
use std::time::Duration;

/// A clean git checkout with one commit, and its HEAD.
fn clean_checkout(parent: &std::path::Path) -> (std::path::PathBuf, String) {
    let repo = parent.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .expect("git runs")
                .status
                .success(),
            "git {args:?} failed"
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "seed"]);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .expect("git runs");
    let rev = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (repo, rev)
}

/// A sweep that completed but could not be saved to disk must fail the run,
/// not be swallowed as a printed warning on a stream the desktop app never
/// shows. `out_dir` is what makes a run recoverable by `--resume`, so a report
/// that did not reach disk is a loss of paid work, not a hiccup.
///
/// The sweep itself is allowed to fail (the model does not exist); what matters
/// is that it produces an outcome, that outcome is written, the write fails, and
/// the run reports the failure instead of returning Ok.
#[tokio::test]
async fn a_run_whose_completed_sweep_could_not_be_saved_fails_instead_of_swallowing_it() {
    let parent = std::env::temp_dir()
        .join("bugsleuth-persist-tests")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    let (repo, _rev) = clean_checkout(&parent);

    // `out_dir` is a plain file, so `create_dir_all` inside `write_report`
    // cannot turn it into a directory and every report fails to save.
    let blocker = parent.join("blocker-file");
    std::fs::write(&blocker, "not a directory").expect("write blocker");

    let plan = plan(&Config {
        models: vec![ModelPlan {
            id: "claude:no-such-model".to_string(),
            lanes: vec!["correctness".to_string()],
            effort: String::new(),
            use_agents: false,
            passes: 1,
        }],
    })
    .expect("plan");

    let result = run(
        &plan,
        RunOptions {
            repo: &repo,
            scope: None,
            triage_model: "",
            cancel: Default::default(),
            max_turns: 1,
            timeout: Duration::from_secs(2),
            api_key: None,
            out_dir: Some(&blocker),
            resume: false,
            progress: None,
        },
    )
    .await;

    let _ = std::fs::remove_dir_all(&parent);
    let error = match result {
        Ok(_) => String::new(),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("could not be saved"),
        "the persistence failure was swallowed: {error}"
    );
}
