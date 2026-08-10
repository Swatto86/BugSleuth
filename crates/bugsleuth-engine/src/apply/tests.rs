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
        prompt: "fix it",
        timeout: Duration::from_secs(1),
        max_turns: 1,
        cancel: crate::cancel::Cancel::new(),
        push: false,
        tag: false,
    })
    .await
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(error.contains("not a git repository"), "{error}");
}
