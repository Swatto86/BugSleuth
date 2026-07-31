//! The M0 harness: run one lane against one repository with one model, verify
//! every anchor, and print what survived.

mod brief;
mod report;
mod sweep;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use bugsleuth_domain::Lane;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bugsleuth",
    about = "Adversarial cross-vendor code review",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one lane against a repository.
    Sweep(SweepArgs),
    /// Check that a configured provider CLI can be found and run.
    Preflight,
}

#[derive(Parser)]
struct SweepArgs {
    /// Repository to review.
    #[arg(long)]
    repo: PathBuf,
    /// Review mandate: correctness, security, contract, or ux.
    #[arg(long)]
    lane: Lane,
    /// Model alias or id, e.g. sonnet, opus, haiku.
    #[arg(long, default_value = "sonnet")]
    model: String,
    /// Limit the review to these paths (passed to the model as guidance).
    #[arg(long)]
    scope: Option<String>,
    /// Hard ceiling on agent turns, the main guard against one lane consuming
    /// the whole subscription quota.
    #[arg(long, default_value_t = 30)]
    max_turns: u32,
    #[arg(long, default_value_t = 900)]
    timeout_secs: u64,
    /// Write the machine-readable report here in addition to printing text.
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Use an API key instead of the signed-in subscription session. The key is
    /// read from `ANTHROPIC_API_KEY`; it is never accepted as an argument, so it
    /// cannot end up in a shell history or a process listing.
    #[arg(long)]
    use_api_key: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Sweep(args) => run_sweep(args).await,
        Command::Preflight => sweep::preflight().await,
    }
}

async fn run_sweep(args: SweepArgs) -> Result<()> {
    let repo = args
        .repo
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot read repository {}: {e}", args.repo.display()))?;

    let api_key = if args.use_api_key {
        let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            anyhow::anyhow!("--use-api-key was given but ANTHROPIC_API_KEY is not set")
        })?;
        Some(key)
    } else {
        None
    };

    let report = sweep::run(sweep::Request {
        repo: &repo,
        lane: args.lane,
        model: &args.model,
        scope: args.scope.as_deref(),
        max_turns: args.max_turns,
        timeout: Duration::from_secs(args.timeout_secs),
        api_key: api_key.as_deref(),
    })
    .await;

    print!("{}", report.to_text());

    if let Some(path) = args.json_out {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&path, json)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }

    // A lane that failed to run must not look like a clean pass to a script.
    if matches!(report.status, report::Status::NotSwept { .. }) {
        std::process::exit(2);
    }
    Ok(())
}
