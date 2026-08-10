//! Isolation: the throwaway checkout an isolated vendor reviews.
//!
//! Split from `tests.rs` at the hard line cap, along the seam already there:
//! those tests cover vendor parsing, argv and report labels, and these cover
//! what has to be true of the checkout before a vendor is allowed near it.
//! Both refusals here are gaps a reader would otherwise never hear about.

use super::*;

#[tokio::test]
async fn a_kilo_sweep_stops_at_the_preflight_before_doing_any_work() {
    // Whichever way this machine is configured, a Kilo sweep must consult
    // the preflight before discovering a provider or building a worktree.
    // The two outcomes are asserted against each other rather than against
    // a fixed expectation, because the honest answer depends on the config:
    // a refusal must name the open tool, and a pass must mean the config
    // really denies all host capabilities. Both halves of `permission_gap`
    // are tested directly, against written configs, in the provider crate.
    let report = run(Request {
        repo: Path::new("."),
        lane: Lane::Security,
        model: "kilo:some/model",
        scope: None,
        effort: "",
        max_turns: 1,
        timeout: Duration::from_secs(1),
        api_key: None,
        binary: None,
    })
    .await;

    match (kilo::preflight::permission_gap(), report.status) {
        (Some(gap), Status::NotSwept { reason }) => assert!(
            reason.contains(&gap),
            "the refusal did not carry the permission gap: {reason}"
        ),
        (Some(gap), other) => {
            panic!("permissions are open ({gap}) but the sweep was not refused: {other:?}")
        }
        // Permissions safe: the preflight is satisfied and the sweep proceeds
        // to the next failure, which without a Kilo binary is a real one.
        (None, _) => {}
    }
}

/// Kilo must refuse a partial checkout rather than review it quietly.
///
/// `git worktree add` checks out gitlinks without initializing their contents,
/// so Kilo's throwaway worktree holds an empty directory where an initialized
/// submodule's source is. Claude and Codex read the main checkout and see it.
/// The result came back as an ordinary swept lane that had reviewed less code,
/// which is precisely the silent gap this tool exists to prevent.
#[tokio::test]
async fn kilo_does_not_silently_omit_submodule_contents() {
    let dir = std::env::temp_dir()
        .join("bugsleuth-submodule-sweep")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let parent = dir.join("parent");
    let child = dir.join("child");
    std::fs::create_dir_all(&parent).expect("parent");
    std::fs::create_dir_all(&child).expect("child");
    let git = |cwd: &Path, args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !git(&child, &["init", "-q"]) {
        return; // no usable git here
    }
    for cwd in [&child, &parent] {
        let _ = git(cwd, &["init", "-q"]);
        let _ = git(cwd, &["config", "user.email", "t@example.invalid"]);
        let _ = git(cwd, &["config", "user.name", "test"]);
        let _ = git(cwd, &["config", "protocol.file.allow", "always"]);
    }
    std::fs::write(child.join("lib.rs"), "fn defective() {}\n").expect("write");
    let _ = git(&child, &["add", "-A"]);
    let _ = git(&child, &["commit", "-qm", "child"]);
    std::fs::write(parent.join("main.rs"), "fn main() {}\n").expect("write");
    let _ = git(&parent, &["add", "-A"]);
    let _ = git(&parent, &["commit", "-qm", "parent"]);
    let child_url = child.to_string_lossy().replace('\\', "/");
    // `-c` rather than repository config: git applies the protocol allowlist to
    // the clone subprocess, which does not inherit the parent's config here.
    assert!(
        git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &child_url,
                "vendor/lib",
            ],
        ),
        "the submodule fixture was not created, so this test would prove nothing"
    );
    let _ = git(&parent, &["commit", "-qm", "add submodule"]);
    // The gitlink is really in the index, or the checkout below has nothing to
    // be partial about and the assertion at the end passes on an empty scan.
    let staged = std::process::Command::new("git")
        .args(["ls-files", "--stage"])
        .current_dir(&parent)
        .output()
        .expect("ls-files");
    assert!(
        String::from_utf8_lossy(&staged.stdout).contains("160000 "),
        "the fixture has no gitlink, so there is no partial checkout to detect"
    );

    let report = run(Request {
        repo: &parent,
        lane: Lane::Correctness,
        model: "kilo:",
        scope: None,
        effort: "",
        max_turns: 1,
        // Short deliberately: the guard fires before any provider is invoked, so
        // a regression that reaches the real CLI costs seconds rather than a
        // minute of somebody's subscription quota.
        timeout: Duration::from_secs(5),
        api_key: None,
        binary: None,
    })
    .await;

    // The exact reason. Asserting only `NotSwept` would be satisfied by the
    // Kilo permission precheck, a missing CLI, or any other refusal that has
    // nothing to do with submodules.
    let Status::NotSwept { reason } = &report.status else {
        panic!("a partial checkout was reviewed as a normal sweep: {report:?}");
    };
    assert!(
        reason.contains("does not initialize submodules"),
        "the lane was refused for some other reason: {reason}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
