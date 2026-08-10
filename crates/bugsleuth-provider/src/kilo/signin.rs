//! Proving this machine holds a usable Kilo session.
//!
//! Split from `kilo.rs` at the hard line cap, along the seam already there: that
//! file runs sweeps, this one runs the smallest possible invocation to find out
//! whether a sweep could run at all.
//!
//! Kilo authenticates **per route**, not per vendor. An Ollama model, an
//! OpenRouter key, a Kilo plan and the configured default are each available or
//! not independently, so there is no single answer to "is Kilo signed in".

use std::path::PathBuf;

use super::sweep_environment;
use super::{KiloSweep, build_args, discover, events, repair};

/// Prove the machine holds a Kilo session, by using it.
///
/// Checks Kilo's *configured default* route, which is what the all-vendor
/// diagnostic is about. A run that selected a specific model must use
/// [`signin_for`] instead: the routes are not interchangeable.
pub async fn signin() -> crate::signin::SignIn {
    signin_for("", "", None).await
}

/// Prove the machine can reach one specific Kilo model, by using it.
///
/// Kilo authentication is per route, not per vendor: an Ollama model, an
/// OpenRouter key, a Kilo plan and the configured default can each be available
/// or not independently. Checking the default therefore gated a run on an
/// invocation it was never going to make — a signed-out cloud default blocked
/// every lane before a working local model was tried, and the inverse passed the
/// pre-check and failed the selected route afterwards.
///
/// The argv and environment come from [`build_args`] and [`sweep_environment`],
/// the same two a sweep uses. A check that assembles its own flags is not
/// checking the work.
pub async fn signin_for(model: &str, effort: &str, binary: Option<&str>) -> crate::signin::SignIn {
    let binary = match binary {
        Some(path) => PathBuf::from(path),
        None => match discover::resolve_binary() {
            Some(path) => path,
            None => {
                return crate::signin::SignIn::Failed(
                    "the kilo CLI could not be found".to_string(),
                );
            }
        },
    };
    // Pointed at an empty private directory, exactly as the repair pass is and
    // for the same reason: Kilo runs with `--auto`, so any tool call it makes is
    // approved without asking, and it cannot be restricted for a single
    // invocation. Without `--dir` this would run against whatever directory the
    // app happened to start in — and Kilo resolves some paths from `--dir`
    // rather than the process cwd, so pinning it is the only way to be sure.
    let Some(sandbox) = repair::empty_dir() else {
        return crate::signin::SignIn::Failed(
            "could not create a private directory to ask Kilo from".to_string(),
        );
    };
    let _guard = repair::Cleanup(sandbox.clone());

    // Built by the sweep's own argument builder, from the sweep's own struct.
    // This used to assemble the flags by hand, so the model and effort a lane
    // would really pass were simply absent and the check exercised a different
    // invocation from the one it gates.
    let args = build_args(&KiloSweep {
        worktree: &sandbox,
        model,
        effort,
        brief: "",
        timeout: crate::signin::TIMEOUT,
        binary: None,
    });

    crate::signin::one_shot(
        &binary.to_string_lossy(),
        &args,
        &sweep_environment(),
        "kilo",
        events::assistant_text,
    )
    .await
}
