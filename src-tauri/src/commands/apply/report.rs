//! What the window is told an apply did.
//!
//! Split from the command beside it because it is a different job: that one
//! validates a request and spawns the work, this one turns a finished report
//! into the text a person reads. The rule every function here follows is the
//! same, and it is the reason the whole project exists — say what was observed,
//! never what was intended, and never let a step that quietly did not happen
//! read like one that did.

/// The model's account, under what git says actually happened.
///
/// git first and deliberately: the account is a claim, and a reader who has
/// already seen the file list reads the claim against it rather than instead
/// of it. A model that reports three fixes and touched nothing is the case this
/// ordering exists for.
pub(super) fn describe(report: &bugsleuth_engine::apply::ApplyReport) -> String {
    let mut out = String::new();
    if report.changed_files.is_empty() {
        out.push_str(
            "git reports no files changed. Whatever the model says below, nothing in \
                      the repository is different.\n\n",
        );
    } else {
        let files = report.changed_files.len();
        out.push_str(&format!(
            "{files} file{} changed, {}:\n",
            if files == 1 { "" } else { "s" },
            match report.commits {
                0 => "none of it committed".to_string(),
                1 => "in 1 commit".to_string(),
                n => format!("in {n} commits"),
            },
        ));
        for file in &report.changed_files {
            out.push_str(&format!("  {file}\n"));
        }
        out.push_str(
            "\nNothing here has been verified. Read the diff before you keep it: `git diff` for \
             what is uncommitted, `git log -p` for what was committed.\n\n",
        );
    }
    // Said plainly, because rewriting someone's commits without telling them is
    // worse than leaving the trailer on.
    if !report.stripped.is_empty() {
        out.push_str(&format!(
            "{}, so this tool does not sign your history:\n",
            match report.stripped.len() {
                1 => "Removed a tool authorship trailer from 1 commit".to_string(),
                n => format!("Removed tool authorship trailers from {n} commits"),
            }
        ));
        for subject in &report.stripped {
            out.push_str(&format!("  {subject}\n"));
        }
        out.push_str(
            "\nThe changes themselves are untouched — only the trailer lines are gone.\n\n",
        );
    }

    // What could not be rewritten, because it was already published or HEAD was
    // detached. Reported rather than forced: forking history to tidy a trailer
    // is a worse outcome than the trailer.
    if !report.attributed.is_empty() {
        out.push_str(&format!(
            "{}:\n",
            match report.attributed.len() {
                1 => "1 commit credits a tool for the work".to_string(),
                n => format!("{n} commits credit a tool for the work"),
            }
        ));
        for subject in &report.attributed {
            out.push_str(&format!("  {subject}\n"));
        }
        out.push_str(
            "\nStrip the trailer before pushing if your project does not want it — once it is \
             published it is in the history for good.\n\n",
        );
    }

    out.push_str(&describe_push(&report.push));
    out.push_str(&describe_tag(&report.tag));

    out.push_str("The model's own account:\n\n");
    out.push_str(report.text.trim());
    out.push('\n');
    out
}

/// What became of publishing, in the same pane and by the same rule as the
/// rest: say what happened, not what was intended.
///
/// A refusal is spelled out rather than left silent. "Push after applying" is a
/// setting someone turns on and then stops thinking about, and a tool that
/// quietly declines is indistinguishable from one that quietly succeeded —
/// until the day they find the branch was never published.
fn describe_push(outcome: &bugsleuth_engine::apply::PushOutcome) -> String {
    use bugsleuth_engine::apply::PushOutcome as P;
    match outcome {
        // The setting is off. Saying so on every apply would be noise.
        P::NotRequested => String::new(),
        P::NothingToPush => {
            "Nothing was pushed: the model committed nothing, so there is nothing to publish.\n\n"
                .to_string()
        }
        P::Refused(reason) => format!("Not pushed. {reason}\n\n"),
        P::Pushed { branch, upstream } => {
            format!(
                "Pushed {branch} to {upstream}. These commits are published now — a later \
                     rewrite does not recall them, so read the diff there before anyone builds \
                     on it.\n\n"
            )
        }
        P::Failed(error) => format!(
            "The push was rejected and nothing was published: {error}\n\nThe commits are safe in \
             your repository. This is left for you on purpose — recovering from a rejected push \
             means a fetch and a rebase, or a force, and neither is a choice a tool should make \
             with your history.\n\n"
        ),
        P::Unknown {
            branch,
            upstream,
            error,
        } => format!(
            "The push to {upstream} errored and the remote could not be confirmed, so whether \
             {branch} was published is unknown: {error}\n\nCheck {upstream} before you retry — the \
             commits may already be there, and pushing again is not automatic here because it \
             could publish work twice.\n\n"
        ),
    }
}

/// What became of tagging, by the same rule as the push above: a refusal is
/// spelled out, because a release that quietly never happened reads exactly
/// like one that did until someone goes looking for it.
fn describe_tag(outcome: &bugsleuth_engine::apply::TagOutcome) -> String {
    use bugsleuth_engine::apply::TagOutcome as T;
    match outcome {
        T::NotRequested => String::new(),
        // The push section directly above already said why nothing was
        // published, so this does not repeat it — only why that also means no
        // release, which is not obvious from a message about a rejected push.
        T::NotPushed => {
            "No release was tagged, because nothing was published to tag.\n\n".to_string()
        }
        T::Refused(reason) => format!("No release was tagged. {reason}\n\n"),
        T::Tagged { tag, remote } => format!(
            "Tagged {tag} and pushed it to {remote}, which is what starts that repository's \
             release workflow. Nothing here checked that the workflow passed, or that what it \
             builds is correct — watch the run, and remember a published release is not \
             something a later commit takes back.\n\n"
        ),
        T::Failed(error) => format!(
            "The tag was rejected and no release was started: {error}\n\nThe commits are pushed \
             either way. Tag it by hand once you know why.\n\n"
        ),
        T::Unknown { tag, remote, error } => format!(
            "The tag push to {remote} errored and the remote could not be confirmed, so whether \
             {tag} started a release is unknown: {error}\n\nCheck {remote} for {tag} before you \
             retry — the release may already be running, and tagging again is not automatic here \
             because starting a release twice is not something a later commit takes back.\n\n"
        ),
    }
}

#[cfg(test)]
#[path = "report/tests.rs"]
mod tests;
