//! The command-line surface.
//!
//! Argument definitions only. Everything here is `pub(crate)` because the
//! command implementations in `main` read these fields directly; nothing else
//! should.

use std::path::PathBuf;

use bugsleuth_domain::Lane;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bugsleuth",
    about = "Adversarial cross-vendor code review",
    version
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Run one lane against a repository.
    Sweep(SweepArgs),
    /// Run every configured (model x lane) pair and produce one merged report.
    Run(RunArgs),
    /// Merge several sweep reports into one ranked list of distinct defects.
    Judge(JudgeArgs),
    /// Check that a configured provider CLI can be found and run.
    Preflight,
}

#[derive(Parser)]
pub(crate) struct RunArgs {
    /// Repository to review.
    #[arg(long)]
    pub(crate) repo: PathBuf,
    /// JSON file assigning lanes to models.
    #[arg(long)]
    pub(crate) config: PathBuf,
    /// Limit the review to these paths.
    #[arg(long)]
    pub(crate) scope: Option<String>,
    /// Claude-only hard ceiling on agent turns. In a mixed run this applies
    /// only to Claude; Codex and Kilo use the per-invocation timeout instead.
    #[arg(long)]
    pub(crate) max_turns: Option<u32>,
    /// Per-sweep timeout. Generous on purpose: on a real crate, Claude finished
    /// in about five minutes while Codex was still working at twenty-five and
    /// was killed. Vendors differ by far more than seems reasonable, and a
    /// too-short timeout throws away a sweep that was nearly done.
    #[arg(long, default_value_t = 2700)]
    pub(crate) timeout_secs: u64,
    /// Directory for each individual sweep's JSON, so a run that dies part way
    /// through does not discard the sweeps already paid for.
    #[arg(long)]
    pub(crate) out_dir: Option<PathBuf>,
    /// Reuse successful sweeps already in --out-dir instead of paying for them
    /// again. Failed sweeps are retried.
    #[arg(long, requires = "out_dir")]
    pub(crate) resume: bool,
    #[arg(long)]
    pub(crate) use_api_key: bool,
    /// Model that re-grades every severity once the sweeps are merged, seeing
    /// the whole list at once. Each sweep otherwise grades its own findings in
    /// isolation, which measured wrong 6 times in 14. Pass an empty string to
    /// skip the pass and keep each model's own grade.
    #[arg(long, default_value = "haiku")]
    pub(crate) triage_model: String,
    /// Directory to write fix prompts into: `fix-prompt.md` with everything,
    /// plus `fix-prompt-01.md` onward, one self-contained prompt per defect for
    /// a model that cannot hold the whole thing.
    #[arg(long)]
    pub(crate) prompt_out: Option<PathBuf>,
}

#[derive(Parser)]
pub(crate) struct JudgeArgs {
    /// Sweep report JSON files, as written by `sweep --json-out`.
    #[arg(required = true)]
    pub(crate) reports: Vec<PathBuf>,
    /// Write the merged report here as JSON.
    #[arg(long)]
    pub(crate) json_out: Option<PathBuf>,
    /// Directory to write fix prompts into: `fix-prompt.md` with everything,
    /// plus one self-contained `fix-prompt-NN.md` per defect.
    #[arg(long)]
    pub(crate) prompt_out: Option<PathBuf>,
    /// Repository the defects are in. Named in the prompt so the agent knows
    /// what it is working on.
    #[arg(long, default_value = ".")]
    pub(crate) repo: PathBuf,
}

#[derive(Parser)]
pub(crate) struct SweepArgs {
    /// Repository to review.
    #[arg(long)]
    pub(crate) repo: PathBuf,
    /// Review mandate: correctness, security, contract, ux, or gate.
    #[arg(long)]
    pub(crate) lane: Lane,
    /// Model alias or id, e.g. sonnet, opus, haiku.
    #[arg(long, default_value = "sonnet")]
    pub(crate) model: String,
    /// Reasoning effort. Accepted values depend on the vendor; omitting it
    /// leaves the vendor's own default in place.
    #[arg(long, default_value = "")]
    pub(crate) effort: String,
    /// Limit the review to these paths (passed to the model as guidance).
    #[arg(long)]
    pub(crate) scope: Option<String>,
    /// Claude-only hard ceiling on agent turns. Codex and Kilo instead have a
    /// per-invocation timeout plus one bounded recovery attempt.
    #[arg(long)]
    pub(crate) max_turns: Option<u32>,
    #[arg(long, default_value_t = 900)]
    pub(crate) timeout_secs: u64,
    /// Write the machine-readable report here in addition to printing text.
    #[arg(long)]
    pub(crate) json_out: Option<PathBuf>,
    /// Use an API key instead of the signed-in subscription session. The key is
    /// read from `ANTHROPIC_API_KEY`; it is never accepted as an argument, so it
    /// cannot end up in a shell history or a process listing.
    #[arg(long)]
    pub(crate) use_api_key: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help text names every lane there is.
    ///
    /// Two hand-written lists of the same set, one in a doc comment and one in
    /// `Lane::ALL`. Nothing makes them agree: adding a lane leaves the help
    /// quietly advertising a smaller product, and the reader has no way to know
    /// the option they want exists. This is the same "one rule written down
    /// twice" the contract lane hunts for, in our own argument parser.
    #[test]
    fn the_lane_help_lists_every_lane() {
        let help = <SweepArgs as clap::CommandFactory>::command()
            .render_long_help()
            .to_string();
        for lane in Lane::ALL {
            assert!(
                help.contains(lane.slug()),
                "`--lane` help does not mention {}",
                lane.slug()
            );
        }
    }
}
