//! The M0 harness: run one lane against one repository with one model, verify
//! every anchor, and print what survived.

mod brief;
mod prove;
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
    /// Ask a model to demonstrate a defect with a failing test, then check the
    /// attempt by running the tests independently.
    Prove(ProveArgs),
    /// Check that a configured provider CLI can be found and run.
    Preflight,
}

#[derive(Parser)]
struct ProveArgs {
    /// Repository containing the defect. It is never modified — the attempt runs
    /// in a throwaway git worktree made from it.
    #[arg(long)]
    repo: PathBuf,
    /// Commit to base the worktree on.
    #[arg(long, default_value = "HEAD")]
    commit: String,
    /// Description of the defect to prove. Use `--defect-file` for a long one.
    #[arg(long, conflicts_with = "defect_file")]
    defect: Option<String>,
    /// Read the defect description from a file.
    #[arg(long)]
    defect_file: Option<PathBuf>,
    #[arg(long, default_value = "sonnet")]
    model: String,
    /// Command that runs the tests, e.g. "cargo test -p mycrate --lib".
    #[arg(long, default_value = "cargo test")]
    test_command: String,
    /// Name for the throwaway branch and worktree directory.
    #[arg(long, default_value = "proof")]
    label: String,
    #[arg(long, default_value_t = 40)]
    max_turns: u32,
    #[arg(long, default_value_t = 1200)]
    timeout_secs: u64,
    #[arg(long, default_value_t = 600)]
    test_timeout_secs: u64,
    /// Write the model's added test as a patch here, so it can be replayed
    /// against fixed code.
    #[arg(long)]
    patch_out: Option<PathBuf>,
    #[arg(long)]
    use_api_key: bool,
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
        Command::Prove(args) => run_prove(args).await,
        Command::Preflight => sweep::preflight().await,
    }
}

async fn run_prove(args: ProveArgs) -> Result<()> {
    let repo = args
        .repo
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot read repository {}: {e}", args.repo.display()))?;

    let defect = match (&args.defect, &args.defect_file) {
        (Some(text), _) => text.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?,
        (None, None) => anyhow::bail!("give the defect with --defect or --defect-file"),
    };

    let report = prove::attempt(prove::Attempt {
        repo: &repo,
        commit: &args.commit,
        model: &args.model,
        brief: &brief::proof(&defect, &args.test_command),
        test_command: &args.test_command,
        max_turns: args.max_turns,
        timeout: Duration::from_secs(args.timeout_secs),
        test_timeout: Duration::from_secs(args.test_timeout_secs),
        api_key: api_key(args.use_api_key)?.as_deref(),
        label: &args.label,
    })
    .await?;

    println!("{}", report.to_text());

    if let Some(path) = args.patch_out
        && !report.patch.is_empty()
    {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &report.patch)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    if !report.verdict.is_proof() {
        std::process::exit(3);
    }
    Ok(())
}

/// The key is only ever read from the environment. Passing it as an argument
/// would put it in shell history and in the process list.
fn api_key(requested: bool) -> Result<Option<String>> {
    if !requested {
        return Ok(None);
    }
    std::env::var("ANTHROPIC_API_KEY")
        .map(Some)
        .map_err(|_| anyhow::anyhow!("--use-api-key was given but ANTHROPIC_API_KEY is not set"))
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
        // Create the parent directory rather than failing after the sweep has
        // already been paid for: losing a completed run's output to a missing
        // folder wastes real quota.
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", parent.display()))?;
        }
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
