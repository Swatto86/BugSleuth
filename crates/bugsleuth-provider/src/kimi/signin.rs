//! Proving this machine holds a usable Kimi session.
//!
//! The smallest real invocation, through the same argv builder a sweep uses.
//! A probe that assembles its own flags is not checking the work — that was a
//! live defect in the Kilo adapter, where the pre-check tested a route no lane
//! would ever take.

use std::path::PathBuf;

use super::{KimiSweep, brief_file, build_args, discover};

/// Prove the machine can reach one specific Kimi model, by using it.
///
/// The model matters. A Kimi subscription and a bring-your-own-key setup do not
/// reach the same set, so "is Kimi signed in" has no single answer — the only
/// useful question is whether the model this run selected will answer.
pub async fn signin_for(model: &str, binary: Option<&str>) -> crate::signin::SignIn {
    let binary = match binary {
        Some(path) => PathBuf::from(path),
        None => match discover::resolve_binary() {
            Some(path) => path,
            None => {
                return crate::signin::SignIn::Failed(
                    "the kimi CLI could not be found".to_string(),
                );
            }
        },
    };

    // A one-word brief, delivered the way a real brief is. Pointing the probe
    // at a file it must open also proves the `--add-dir` grant works, which is
    // the part most likely to break and the part a sweep depends on.
    let brief = match brief_file::BriefFile::write(
        "Reply with the single word OK. Do not read any files. Do not use any tools.",
    ) {
        Ok(brief) => brief,
        Err(error) => return crate::signin::SignIn::Failed(error.to_string()),
    };

    let args = build_args(
        &KimiSweep {
            // Its own directory: the probe must not touch a repository, and
            // the brief's directory is one this process just created.
            worktree: brief.dir(),
            model,
            brief: "",
            timeout: crate::signin::TIMEOUT,
            binary: None,
        },
        &brief,
    );

    crate::signin::one_shot(
        &binary.to_string_lossy(),
        &args,
        &[],
        "kimi",
        str::to_string,
    )
    .await
}

/// The all-vendor diagnostic, which asks about the configured default model.
pub async fn signin() -> crate::signin::SignIn {
    signin_for("", None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kimi::BASE_FLAGS;

    /// The probe runs the flags a sweep runs.
    ///
    /// Not a comparison of two constants — both sides read `BASE_FLAGS`, so
    /// that would agree by construction. What must hold is that the probe's
    /// argv carries the model it was asked about and the prompt pointer, which
    /// is what a sweep's argv carries.
    #[tokio::test]
    async fn the_probe_invokes_the_cli_the_way_a_sweep_does() {
        let dir = std::env::temp_dir()
            .join("bugsleuth-kimi-probe")
            .join(format!("{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let recorded = dir.join("argv.txt");

        #[cfg(windows)]
        let stub = {
            let path = dir.join("kimi.cmd");
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\necho %* > \"{}\"\r\nexit /b 1\r\n",
                    recorded.display()
                ),
            )
            .expect("write stub");
            path
        };
        #[cfg(not(windows))]
        let stub = {
            use std::os::unix::fs::PermissionsExt as _;
            let path = dir.join("kimi.sh");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"$@\" > '{}'\nexit 1\n",
                    recorded.display()
                ),
            )
            .expect("write stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            path
        };

        let outcome = signin_for("kimi-k3", Some(&stub.to_string_lossy())).await;
        assert!(
            !outcome.usable(),
            "a stub that exits non-zero must not read as a working session"
        );

        let argv = std::fs::read_to_string(&recorded).expect("the stub recorded no argv");
        for expected in BASE_FLAGS {
            assert!(
                argv.contains(expected),
                "the probe dropped a flag every sweep passes ({expected}): {argv}"
            );
        }
        assert!(
            argv.contains("-m kimi-k3"),
            "the probe asked about a different model from the one selected: {argv}"
        );
        assert!(
            argv.contains("--add-dir"),
            "the probe cannot read the brief it points at: {argv}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
