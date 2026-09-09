//! Handing one defect at a time to the model that fixes it.
//!
//! Split from `apply.rs` at the function-length and file-length limits, along
//! the seam already there: everything in the parent prepares the repository or
//! reports on it afterwards, and this is the part that spends money and edits
//! the user's code.

use anyhow::Context as _;

use super::journal::Journal;
use super::messages::{interrupted_message, stopped_message};
use super::observed::changed_since;
use super::preflight::refuse_if_dirty;
use super::steps::Step;
use super::{ApplyEvent, ApplyRequest, Baseline, emit, failure_message, failure_message_unknown};
use crate::sweep::Vendor;

/// Fix every defect that is not already recorded as done.
///
/// Split out of [`apply`] at the function-length limit, along the seam that was
/// already there: everything above it prepares and everything below it reports,
/// and this is the part that spends money. Returns when the last defect is
/// committed and recorded; bails on the first that fails, by which point the
/// journal already holds every defect before it.
pub(super) async fn fix_each(
    request: &ApplyRequest<'_>,
    vendor: Vendor,
    model: &str,
    work: &[Step],
    progress: &mut Journal,
    base: &Baseline,
) -> anyhow::Result<()> {
    let repo = request.repo;
    let total = work.len();
    let already = progress.completed();
    for (index, step) in work.iter().enumerate() {
        if progress.is_done(step.position) {
            continue;
        }
        if request.cancel.stopped() {
            anyhow::bail!(stopped_message(progress, total, repo, base));
        }
        // Between defects, not only at the start. The model is asked to commit
        // each fix, so a dirty tree here is the previous defect left half done —
        // and the next defect's changes would then be indistinguishable from it,
        // which is the exact confusion the clean-tree rule exists to prevent.
        if index > 0 || already > 0 {
            refuse_if_dirty(repo).with_context(|| {
                format!(
                    "{} of {total} defects were fixed and committed before this. The working \
                     tree is not clean, so the run stopped rather than mixing the next fix \
                     into what is already there; commit or revert those changes and resume.",
                    progress.completed()
                )
            })?;
        }
        emit(
            &request.progress,
            ApplyEvent::DefectStarted {
                position: step.position,
                done: progress.completed(),
                defects: total,
            },
        );

        let provider = run_provider(request, vendor, model, &step.prompt);
        tokio::pin!(provider);
        let attempt = tokio::select! {
            biased;
            () = request.cancel.cancelled() => {
                anyhow::bail!(stopped_message(progress, total, repo, base));
            }
            attempt = &mut provider => attempt,
        };

        // A failure is not "nothing happened". The invocation is killed on
        // timeout and can fail after the model has already rewritten half the
        // tree, and an error on its own would send someone away believing their
        // repository was untouched. Whatever git can see is named in the error
        // too, along with how far the run got and that it can be picked up.
        let text = match attempt {
            Ok(text) => text,
            Err(error) => {
                // If git itself cannot be read afterwards, say the state is
                // unknown rather than claim nothing changed.
                let observed = match changed_since(repo, base) {
                    Ok(changed) => failure_message(&error.to_string(), &changed),
                    Err(_) => failure_message_unknown(&error.to_string()),
                };
                let detail = interrupted_message(&error, progress, total);
                anyhow::bail!("{observed}\n\n{detail}");
            }
        };
        progress.record(step.position, &text)?;
        emit(
            &request.progress,
            ApplyEvent::DefectFinished {
                position: step.position,
                done: progress.completed(),
                defects: total,
            },
        );
    }

    Ok(())
}

async fn run_provider(
    request: &ApplyRequest<'_>,
    vendor: Vendor,
    model: &str,
    prompt: &str,
) -> Result<String, bugsleuth_provider::ProviderError> {
    match vendor {
        Vendor::Claude => {
            bugsleuth_provider::claude::apply(bugsleuth_provider::claude::ApplyRequest {
                repo: request.repo,
                model,
                effort: request.effort,
                prompt,
                timeout: request.timeout,
                max_turns: request.max_turns,
                binary: None,
            })
            .await
        }
        Vendor::Codex => {
            bugsleuth_provider::codex::apply(
                request.repo,
                model,
                request.effort,
                prompt,
                request.timeout,
            )
            .await
        }
        Vendor::OpenCode => {
            bugsleuth_provider::opencode::apply(
                request.repo,
                model,
                request.effort,
                prompt,
                request.timeout,
            )
            .await
        }
        Vendor::Cursor => {
            bugsleuth_provider::cursor::apply(
                request.repo,
                model,
                request.effort,
                prompt,
                request.timeout,
            )
            .await
        }
    }
}
