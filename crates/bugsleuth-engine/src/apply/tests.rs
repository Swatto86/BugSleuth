//! Tests for the apply orchestration, in their own file only because the module
//! plus its tests crossed the hard line cap — the same split the submodules
//! beside it already use.

use super::*;

#[test]
fn a_failed_apply_still_says_what_it_had_already_changed() {
    // The timeout case: the CLI is killed, and everything it wrote before
    // that is still on disk. Reporting only the error would send someone
    // away believing their repository was untouched.
    let text = failure_message("the codex CLI timed out", &["src/a.rs".to_string()]);
    assert!(text.contains("timed out"));
    assert!(text.contains("1 file had already changed"), "{text}");
    assert!(text.contains("src/a.rs"));
    assert!(text.contains("git status"));

    // And when nothing changed, it says so rather than staying silent —
    // "the run failed" alone leaves the reader guessing about their tree.
    let clean = failure_message("the kilo CLI exited with code 1", &[]);
    assert!(clean.contains("no files changed"), "{clean}");
}

#[test]
fn an_unreadable_repository_after_a_failure_is_not_reported_as_clean() {
    // When the vendor failed and git could not be read afterwards, the
    // message must not claim the tree is clean — "no files changed" there
    // would be a false all-clear on a repository whose state is unknown.
    let unknown = failure_message_unknown("the codex CLI timed out");
    assert!(unknown.contains("timed out"));
    assert!(
        !unknown.contains("no files changed"),
        "an unknown tree must not read as clean: {unknown}"
    );
    assert!(
        unknown.contains("could not read the repository"),
        "{unknown}"
    );
}

#[test]
fn a_credential_in_an_apply_failure_is_redacted_like_a_sweep_error_is() {
    let jwt = "eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl";
    let shown = failure_message(jwt, &[]);
    assert!(
        shown.contains("<redacted-credential>"),
        "a JWT survived unredacted: {shown}"
    );
    let unknown = failure_message_unknown(jwt);
    assert!(
        unknown.contains("<redacted-credential>"),
        "a JWT survived unredacted: {unknown}"
    );
}

#[test]
fn only_a_push_that_succeeded_can_lead_to_a_tag() {
    // The ordering rule, and the reason it is a function rather than a
    // `match` inside `apply`: nothing else here can exercise it without a
    // model CLI, so the rule that decides whether someone's release
    // pipeline fires would otherwise be the one line no test ever reads.
    let pushed = PushOutcome::Pushed {
        branch: "main".into(),
        upstream: "origin/main".into(),
        remote: "origin".into(),
        oid: "abc123".into(),
    };
    // The exact remote, not the `origin/main` display string, so a remote
    // whose own name contains a slash is not truncated on the way to the tag.
    assert_eq!(to_tag(true, &pushed), Some(("origin", "abc123")));

    // Every other outcome means the commits are not on the remote, so a tag
    // would start a build of a ref the runner cannot fetch.
    for refusal in [
        PushOutcome::NotRequested,
        PushOutcome::NothingToPush,
        PushOutcome::Refused("no upstream".into()),
        PushOutcome::Failed("non-fast-forward".into()),
    ] {
        assert_eq!(
            to_tag(true, &refusal),
            None,
            "a release was tagged after {refusal:?}"
        );
    }

    // And the setting still governs: a successful push is not consent to
    // publish a release on its own.
    assert_eq!(to_tag(false, &pushed), None);
}

/// An orphan branch is unborn even when the repository has other history.
///
/// `rev-list --all --count` answers a question about the whole repository, not
/// about the branch that is checked out, so `git switch --orphan` on a
/// repository with any commit anywhere was reported as a corrupt HEAD and every
/// apply against it refused.
#[test]
fn orphan_branch_with_other_history_is_an_unborn_baseline() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-orphan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git")
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@example.invalid"]);
    run(&["config", "user.name", "test"]);
    std::fs::write(dir.join("a.txt"), "hello\n").expect("write");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "base"]);
    let switched = run(&["switch", "--orphan", "fresh"]);
    if !switched.status.success() {
        return; // git too old for --orphan; the rest of the suite still applies
    }
    // `switch --orphan` keeps the index; a clean orphan branch is the case.
    run(&["rm", "-rq", "--cached", "."]);
    let _ = std::fs::remove_file(dir.join("a.txt"));

    assert!(
        matches!(baseline(&dir), Ok(Baseline::Unborn)),
        "a clean orphan branch is a valid starting point, not a corrupt HEAD"
    );
    assert_eq!(
        observed::range_since(&dir, &Baseline::Unborn).expect("range"),
        None,
        "an orphan branch with no commit of its own has no range to inspect"
    );

    std::fs::write(dir.join("b.txt"), "new\n").expect("write");
    run(&["add", "-A"]);
    run(&["commit", "-qm", "first on the orphan"]);
    assert_eq!(
        observed::range_since(&dir, &Baseline::Unborn).expect("range"),
        Some("HEAD".to_string()),
        "once the orphan branch has a commit, all of HEAD is new"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_repository_without_git_is_refused_before_anything_is_spent() {
    let dir = std::env::temp_dir().join(format!("bugsleuth-apply-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let error = apply(ApplyRequest {
        repo: &dir,
        model: "haiku",
        effort: "",
        prompts: &dir,
        timeout: Duration::from_secs(1),
        max_turns: 1,
        cancel: crate::cancel::Cancel::new(),
        progress: None,
        push: false,
        tag: false,
    })
    .await
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        error.contains("refused without one"),
        "a non-repository was not refused: {error}"
    );
}

/// A `.git` file pointing at a *different* repository must be refused before
/// the provider starts. Apply runs git for baseline, commit counting, attribution
/// and optional push, so it must not operate on a victim repository that the
/// chosen directory only points at — the check uses git's own resolution, which
/// follows the indirection, so a directory whose `.git` resolves elsewhere is
/// rejected as not its own.
#[tokio::test]
async fn a_git_file_pointing_at_another_repository_is_refused_before_anything_is_spent() {
    let root =
        std::env::temp_dir().join(format!("bugsleuth-apply-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let attacker = root.join("attacker");
    let victim = root.join("victim");
    std::fs::create_dir_all(&attacker).expect("mkdir");
    std::fs::create_dir_all(&victim).expect("mkdir");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&victim)
            .output()
            .expect("git")
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@x.invalid"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(victim.join("f.txt"), "x\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);

    // The attacker directory's `.git` is a file pointing at the victim's git
    // directory: git would read the victim's HEAD and refs from here.
    std::fs::write(
        attacker.join(".git"),
        format!("gitdir: {}\n", victim.join(".git").display()),
    )
    .expect("write gitfile");

    let error = apply(ApplyRequest {
        repo: &attacker,
        model: "haiku",
        effort: "",
        prompts: &attacker,
        timeout: Duration::from_secs(1),
        max_turns: 1,
        cancel: crate::cancel::Cancel::new(),
        progress: None,
        push: false,
        tag: false,
    })
    .await
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();

    let _ = std::fs::remove_dir_all(&root);
    assert!(
        error.contains("not an independent git repository"),
        "the cross-repository `.git` indirection was accepted: {error}"
    );
}

/// A resumed apply skips what an earlier attempt already fixed.
///
/// The journal has its own tests, but they prove it records and reloads — not
/// that `apply` actually consults it. The whole claim being made to the user is
/// that pressing Apply after a run that died on a usage limit continues rather
/// than starting over, and that claim lives here, in the loop that decides
/// which work orders to hand to a model.
///
/// No provider runs: the model is deliberately one that cannot be started, so
/// the first defect this reaches fails immediately. Which defect that is, and
/// what the failure says about the ones before it, is exactly what is under
/// test.
#[tokio::test]
async fn a_resumed_apply_continues_at_the_defect_the_last_attempt_died_on() {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-apply-resume-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git")
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "test"]);
    std::fs::write(dir.join("a.txt"), "hello\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);

    // Outside the checkout, as the real run directory is: prompts written into
    // the repository would make its tree dirty and be refused before any of
    // this is reached.
    let prompts = dir.with_extension("prompts");
    let _ = std::fs::remove_dir_all(&prompts);
    std::fs::create_dir_all(&prompts).expect("prompts");
    std::fs::write(prompts.join("fix-prompt-01.md"), "fix the first").expect("write");
    std::fs::write(prompts.join("fix-prompt-02.md"), "fix the second").expect("write");

    let model = "no-such-model-please";
    let request = || bugsleuth_engine_apply_request(&dir, &prompts, model);

    // First attempt: nothing is recorded, so it dies on the first defect and
    // must not claim any progress.
    let first = apply(request())
        .await
        .err()
        .map(|error| error.to_string())
        .expect("no provider is installed, so the apply must fail");
    assert!(
        first.contains("No defect was completed"),
        "a failed first defect claimed progress: {first}"
    );

    // Now stand in for an attempt that got the first defect done and then hit a
    // usage limit, which is what the journal would hold.
    let steps = steps::load(&prompts).expect("work orders");
    let mut journal =
        journal::Journal::open(&prompts, model, &steps, baseline(&dir).expect("base"));
    journal
        .record(1, "fixed the first")
        .expect("record the finished defect");
    drop(journal);

    let second = apply(request())
        .await
        .err()
        .map(|error| error.to_string())
        .expect("no provider is installed, so the apply must fail");
    assert!(
        second.contains("1 of 2 defects are fixed"),
        "the resumed run did not report the earlier attempt's work: {second}"
    );
    assert!(
        second.contains("continues at the 1 still outstanding"),
        "the resumed run did not say what is left: {second}"
    );
    // And it is still recorded afterwards: a second failure must not discard
    // the defect the first attempt paid for.
    assert_eq!(unfinished(&prompts, model), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&prompts);
}

/// Builds the request twice over without repeating nine fields.
fn bugsleuth_engine_apply_request<'a>(
    repo: &'a std::path::Path,
    prompts: &'a std::path::Path,
    model: &'a str,
) -> ApplyRequest<'a> {
    ApplyRequest {
        repo,
        model,
        effort: "",
        prompts,
        timeout: Duration::from_secs(5),
        max_turns: 1,
        cancel: crate::cancel::Cancel::new(),
        progress: None,
        push: false,
        tag: false,
    }
}
