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
fn only_a_push_that_succeeded_can_lead_to_a_tag() {
    // The ordering rule, and the reason it is a function rather than a
    // `match` inside `apply`: nothing else here can exercise it without a
    // model CLI, so the rule that decides whether someone's release
    // pipeline fires would otherwise be the one line no test ever reads.
    let pushed = PushOutcome::Pushed {
        branch: "main".into(),
        upstream: "origin/main".into(),
        remote: "origin".into(),
    };
    // The exact remote, not the `origin/main` display string, so a remote
    // whose own name contains a slash is not truncated on the way to the tag.
    assert_eq!(to_tag(true, &pushed), Some("origin"));

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
