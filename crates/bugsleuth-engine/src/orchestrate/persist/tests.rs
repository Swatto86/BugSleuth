//! Tests for report persistence and resume, in their own file only because
//! the module plus its tests crossed the hard line cap.

use super::super::persist::*;
use bugsleuth_domain::Lane;
use std::sync::OnceLock;
use std::time::Duration;

#[test]
fn a_successful_sweep_is_reused_rather_than_paid_for_twice() {
    let dir = scratch("reuse");
    let report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_report_from_another_review_contract_is_not_reused() {
    let dir = scratch("review-contract");
    let path = dir.join(file_name_for(&unit()));
    let report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());

    let mut stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    stored["bugsleuth_review_contract"] = "older".into();
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
    assert!(reusable(&unit(), &options(&dir, true)).is_none());

    stored
        .as_object_mut()
        .unwrap()
        .remove("bugsleuth_review_contract");
    std::fs::write(&path, serde_json::to_string_pretty(&stored).unwrap()).unwrap();
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_sweep_is_retried_not_reused() {
    // The usual reason a run died is a rate limit, which is exactly the case
    // worth attempting again. Reusing it would make the failure permanent.
    let dir = scratch("retry-failed");
    let report = lane_report(Status::NotSwept {
        reason: "rate limited".into(),
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_reused_unless_resume_was_asked_for() {
    let dir = scratch("no-resume");
    let report = lane_report(Status::Swept {
        turns: None,
        salvaged: false,
    });
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, false)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_truncated_report_is_swept_again_rather_than_failing_the_run() {
    // A run killed mid-write leaves half a file. The right response is to
    // sweep again, not to refuse to start.
    let dir = scratch("truncated");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(file_name_for(&unit())), r#"{"lane":"Corr"#);
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_sweep_writes_to_its_own_file() {
    let a = LaneReport {
        lane: "Correctness".into(),
        model: "claude:sonnet".into(),
        commit: None,
        cache_revision: None,
        scope: None,
        excluded_paths: vec![],
        status: Status::Swept {
            turns: None,
            salvaged: false,
        },
        findings: vec![],
        rejected: vec![],
        usage: None,
    };
    let dir = std::env::temp_dir()
        .join("bugsleuth-orchestrate-tests")
        .join(format!("{}", std::process::id()));
    assert!(write_report(&dir, "correctness-claude-sonnet.json", &a).is_ok());
    assert!(dir.join("correctness-claude-sonnet.json").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) fn unit() -> Unit {
    Unit {
        model: "claude:sonnet".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    }
}

pub(super) fn options<'a>(dir: &'a Path, resume: bool) -> RunOptions<'a> {
    RunOptions {
        // Reuse tests key on a real, clean checkout: resume is only allowed to
        // return a sweep whose recorded revision matches the repository's.
        repo: shared_repo(),
        scope: None,
        max_turns: 10,
        timeout: Duration::from_secs(60),
        api_key: None,
        out_dir: Some(dir),
        resume,
        progress: None,
        triage_model: "",
        cancel: Default::default(),
    }
}

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-resume-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

pub(super) fn lane_report(status: Status) -> LaneReport {
    LaneReport {
        lane: "Correctness".into(),
        model: "claude:sonnet".into(),
        commit: None,
        // Matches the shared repo `options` points at, so a clean unchanged
        // revision is reusable; tests that need otherwise override this.
        cache_revision: Some(head(shared_repo())),
        scope: None,
        excluded_paths: vec![],
        status,
        findings: vec![],
        rejected: vec![],
        usage: None,
    }
}

#[test]
fn a_report_is_replaced_whole_or_not_at_all() {
    // fs::write truncates first, so a process killed mid-write left half a
    // report where a complete one had been. Resume treats an unparseable
    // report as absent, so the cost is paying tens of minutes for the same
    // sweep twice - and losing the first result in the process.
    let dir = scratch("atomic");
    let name = file_name_for(&unit());
    let first = lane_report(Status::Swept {
        turns: Some(1),
        salvaged: false,
    });
    assert!(write_report(&dir, &name, &first).is_ok());

    let second = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    assert!(write_report(&dir, &name, &second).is_ok());

    // The replacement landed, and no staging file was left behind to be
    // read as a report by anything walking the directory.
    let reused = reusable(&unit(), &options(&dir, true));
    assert!(reused.is_some());
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".writing"))
        .collect();
    assert!(leftovers.is_empty(), "staging files left: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A report that reviewed one scope must never be reused for another.
///
/// The defect this covers: resume identified a reusable sweep by lane and model
/// alone. Reviewing `src/moduleA`, then narrowing to `src/moduleB` and pressing
/// Run again — with resume on by default in the app — handed back the moduleA
/// report as a review of moduleB. A lane claiming to have read code it never
/// opened is the one output this tool must never produce.
#[test]
fn a_sweep_of_a_different_scope_is_not_reused() {
    let dir = scratch("scope");
    let mut report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    report.scope = Some("src/moduleA".into());
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());

    let mut narrowed = options(&dir, true);
    narrowed.scope = Some("src/moduleB");
    assert!(
        reusable(&unit(), &narrowed).is_none(),
        "a sweep of moduleA was reused as a review of moduleB"
    );

    let widened = options(&dir, true);
    assert!(
        reusable(&unit(), &widened).is_none(),
        "a scoped sweep was reused for a run over the whole repository"
    );

    let mut same = options(&dir, true);
    same.scope = Some("src/moduleA");
    assert!(
        reusable(&unit(), &same).is_some(),
        "the sweep that did review this scope was thrown away"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same rule on the legacy filename path, which is a second way in.
#[test]
fn the_legacy_name_does_not_smuggle_a_differently_scoped_sweep_back_in() {
    let dir = scratch("scope-legacy");
    let mut report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    report.scope = Some("src/moduleA".into());
    assert!(write_report(&dir, &legacy_file_name_for(&unit()), &report).is_ok());

    let mut narrowed = options(&dir, true);
    narrowed.scope = Some("src/moduleB");
    assert!(
        reusable(&unit(), &narrowed).is_none(),
        "the legacy path reused a sweep of a different scope"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn git(repo: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git runs")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

fn clean_repo(name: &str) -> PathBuf {
    let dir = scratch(&format!("repo-{name}"));
    std::fs::create_dir_all(&dir).expect("mkdir");
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "Test"]);
    std::fs::write(dir.join("seed.txt"), "seed\n").expect("write");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "seed"]);
    dir
}

fn head(repo: &Path) -> String {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// One clean, read-only git repo shared by the reuse tests. They only read its
/// revision, so a single checkout serves them all; tests that mutate a repo
/// make their own with `clean_repo`.
fn shared_repo() -> &'static Path {
    static REPO: OnceLock<PathBuf> = OnceLock::new();
    REPO.get_or_init(|| clean_repo("shared")).as_path()
}

#[test]
fn a_sweep_from_an_earlier_revision_is_not_reused_after_the_checkout_advances() {
    // The defect: resume keyed on lane, model and scope alone, so a sweep run
    // at one commit was handed back after HEAD moved — findings from code no
    // longer there, and blind to code that now is.
    let dir = scratch("stale-rev");
    let repo = clean_repo("stale-rev");
    let mut report = lane_report(Status::Swept {
        turns: Some(3),
        salvaged: false,
    });
    report.cache_revision = Some(head(&repo));
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());

    let mut before = options(&dir, true);
    before.repo = &repo;
    assert!(
        reusable(&unit(), &before).is_some(),
        "a sweep of the current revision should be reused"
    );

    // Advance the checkout, exactly as committing a fix would.
    std::fs::write(repo.join("seed.txt"), "changed\n").expect("write");
    git(&repo, &["commit", "-aqm", "advance"]);
    let mut after = options(&dir, true);
    after.repo = &repo;
    assert!(
        reusable(&unit(), &after).is_none(),
        "a sweep from the previous revision was reused after the checkout advanced"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn a_report_with_no_recorded_revision_is_swept_again() {
    // Reports written before revisions were recorded, and any taken over a
    // dirty tree, carry no marker and must not be trusted to describe the tree.
    let dir = scratch("no-marker");
    let mut report = lane_report(Status::Swept {
        turns: Some(1),
        salvaged: false,
    });
    report.cache_revision = None;
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    assert!(reusable(&unit(), &options(&dir, true)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sweep_is_not_reused_while_the_working_tree_is_dirty() {
    // A dirty tree is not any commit, so nothing can be pinned to it. The sweep
    // is redone rather than reused against edited-but-uncommitted code.
    let dir = scratch("dirty");
    let repo = clean_repo("dirty");
    let mut report = lane_report(Status::Swept {
        turns: Some(2),
        salvaged: false,
    });
    report.cache_revision = Some(head(&repo));
    assert!(write_report(&dir, &file_name_for(&unit()), &report).is_ok());
    std::fs::write(repo.join("uncommitted.txt"), "dirty\n").expect("write");
    let mut o = options(&dir, true);
    o.repo = &repo;
    assert!(
        reusable(&unit(), &o).is_none(),
        "a sweep was reused while the working tree was dirty"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repo);
}
