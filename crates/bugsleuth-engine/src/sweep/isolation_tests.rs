//! Isolation: the throwaway checkout an isolated vendor reviews.
//!
//! Split from `tests.rs` at the hard line cap, along the seam already there:
//! those tests cover vendor parsing, argv and report labels, and these cover
//! what has to be true of the checkout before a vendor is allowed near it.
//! Both refusals here are gaps a reader would otherwise never hear about.

use super::*;

/// The pre-check must ask about the model the run will actually use.
///
/// Kilo authenticates per route: an Ollama model, an OpenRouter key, a Kilo plan
/// and the configured default are each available or not independently. Reducing
/// the plan to "wants Kilo" and asking the default therefore gated every lane on
/// an invocation the run was never going to make — and passed runs whose real
/// route was signed out.
#[tokio::test]
async fn selected_kilo_model_is_the_one_prechecked() {
    let dir = std::env::temp_dir()
        .join("bugsleuth-precheck-route")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    let recorded = dir.join("argv.txt");

    #[cfg(windows)]
    let stub = {
        let path = dir.join("kilo.cmd");
        std::fs::write(
            &path,
            format!(
                "@echo off\r\nfindstr /R \".*\" > nul\r\necho %* > \"{}\"\r\nexit /b 1\r\n",
                recorded.display()
            ),
        )
        .expect("write stub");
        path
    };
    #[cfg(not(windows))]
    let stub = {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("kilo.sh");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncat >/dev/null\necho \"$@\" > '{}'\nexit 1\n",
                recorded.display()
            ),
        )
        .expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    };

    let outcome =
        bugsleuth_provider::kilo::signin_for("ollama/qwen", "high", Some(&stub.to_string_lossy()))
            .await;
    assert!(
        !outcome.usable(),
        "the stub exits non-zero, so this must not read as a working session"
    );

    let argv = std::fs::read_to_string(&recorded).expect("the stub recorded no argv");
    assert!(
        argv.contains("--agent"),
        "the check no longer uses the sweep's own arguments: {argv}"
    );
    assert!(
        argv.contains("-m ollama/qwen"),
        "the pre-check asked about a different route from the one selected: {argv}"
    );
    assert!(
        argv.contains("--variant high"),
        "the pre-check dropped the effort the lane will pass: {argv}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

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

    // The guard itself, not `run()`. Reaching it through `run` puts the Kilo
    // permission precheck in front, so on a machine with no Kilo config the
    // lane is refused for that reason instead and this test would pass or fail
    // on how the host happens to be set up rather than on the defect.
    let refusal = isolate::checkout_for(Vendor::Kilo, &parent)
        .expect_err("a partial checkout was accepted for review");

    // The exact reason. Asserting only that it was refused would be satisfied
    // by a worktree that could not be created at all.
    assert!(
        refusal.contains("does not initialize submodules"),
        "the lane was refused for some other reason: {refusal}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
