//! Proving this machine holds a usable Cursor Agent session.
//!
//! The smallest real invocation, through the same argv builder a sweep uses.
//! `agent status` answers whether a login token exists; it does not prove a
//! model call will succeed. This does.

use super::{CursorSweep, brief_file, build_args, discover};

/// Prove the machine can reach one specific Cursor model, by using it.
pub async fn signin_for(model: &str, binary: Option<&str>) -> crate::signin::SignIn {
    let launch = match binary {
        Some(path) => discover::Launch {
            binary: std::path::PathBuf::from(path),
            prefix: Vec::new(),
        },
        None => match discover::resolve() {
            Some(launch) => launch,
            None => {
                return crate::signin::SignIn::Failed(
                    "the cursor agent CLI could not be found".to_string(),
                );
            }
        },
    };

    let brief = match brief_file::BriefFile::private(
        "Reply with the single word OK. Do not read any other files. Do not use any tools.",
    ) {
        Ok(brief) => brief,
        Err(error) => return crate::signin::SignIn::Failed(error.to_string()),
    };

    let args = build_args(
        &launch.prefix,
        &CursorSweep {
            worktree: brief.workspace(),
            model,
            brief: "",
            timeout: crate::signin::TIMEOUT,
            binary: None,
        },
        &brief,
    );

    crate::signin::one_shot(
        &launch.binary.to_string_lossy(),
        &args,
        brief.workspace(),
        &[],
        "cursor",
        str::to_string,
    )
    .await
}

/// The all-vendor diagnostic, which asks about the configured default model.
pub async fn signin() -> crate::signin::SignIn {
    signin_for("", None).await
}
