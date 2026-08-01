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
    /// Ask a model to demonstrate a defect with a failing test, then check the
    /// attempt by running the tests independently.
    Prove(ProveArgs),
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
    #[arg(long, default_value_t = 40)]
    pub(crate) max_turns: u32,
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
    /// Attempt to prove the top N merged defects with a failing test. 0 (the
    /// default) proves nothing. Each attempt costs a model invocation and a full
    /// test run, so this is the expensive part of a run.
    #[arg(long, default_value_t = 0, requires = "test_command")]
    pub(crate) prove_top: usize,
    /// Command that runs the target's tests, e.g. "cargo test".
    #[arg(long)]
    pub(crate) test_command: Option<String>,
    /// Model used for proof attempts. Kilo cannot prove.
    #[arg(long, default_value = "sonnet")]
    pub(crate) prove_model: String,
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
pub(crate) struct ProveArgs {
    /// Repository containing the defect. It is never modified — the attempt runs
    /// in a throwaway git worktree made from it.
    #[arg(long)]
    pub(crate) repo: PathBuf,
    /// Commit to base the worktree on.
    #[arg(long, default_value = "HEAD")]
    pub(crate) commit: String,
    /// Description of the defect to prove. Use `--defect-file` for a long one.
    #[arg(long, conflicts_with = "defect_file")]
    pub(crate) defect: Option<String>,
    /// Read the defect description from a file.
    #[arg(long)]
    pub(crate) defect_file: Option<PathBuf>,
    #[arg(long, default_value = "sonnet")]
    pub(crate) model: String,
    /// Command that runs the tests, e.g. "cargo test -p mycrate --lib".
    #[arg(long, default_value = "cargo test")]
    pub(crate) test_command: String,
    /// Name for the throwaway branch and worktree directory.
    #[arg(long, default_value = "proof")]
    pub(crate) label: String,
    #[arg(long, default_value_t = 40)]
    pub(crate) max_turns: u32,
    #[arg(long, default_value_t = 1200)]
    pub(crate) timeout_secs: u64,
    #[arg(long, default_value_t = 600)]
    pub(crate) test_timeout_secs: u64,
    /// Write the model's added test as a patch here, so it can be replayed
    /// against fixed code.
    #[arg(long)]
    pub(crate) patch_out: Option<PathBuf>,
    #[arg(long)]
    pub(crate) use_api_key: bool,
}

#[derive(Parser)]
pub(crate) struct SweepArgs {
    /// Repository to review.
    #[arg(long)]
    pub(crate) repo: PathBuf,
    /// Review mandate: correctness, security, contract, or ux.
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
    /// Hard ceiling on agent turns, the main guard against one lane consuming
    /// the whole subscription quota.
    #[arg(long, default_value_t = 30)]
    pub(crate) max_turns: u32,
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
