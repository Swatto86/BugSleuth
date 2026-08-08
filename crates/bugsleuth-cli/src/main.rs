//! The M0 harness: run one lane against one repository with one model, verify
//! every anchor, and print what survived.

mod cli;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use bugsleuth_engine::{brief, merge, orchestrate, plan, prove, report, sweep};
use clap::Parser;

use cli::{Cli, Command, JudgeArgs, ProveArgs, RunArgs, SweepArgs};

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Sweep(args) => run_sweep(args).await,
        Command::Prove(args) => run_prove(args).await,
        Command::Run(args) => run_all(args).await,
        Command::Judge(args) => run_judge(args),
        Command::Preflight => sweep::preflight().await,
    }
}

async fn run_all(args: RunArgs) -> Result<()> {
    let repo = real_path(&args.repo)?;
    let plan = plan::load(&args.config)?;

    // Print progress as it happens rather than after the fact.
    let (progress, mut events) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            eprintln!("{}", orchestrate::progress::describe(&event));
        }
    });

    let report = orchestrate::run(
        &plan,
        orchestrate::RunOptions {
            repo: &repo,
            scope: args.scope.as_deref(),
            max_turns: args.max_turns,
            timeout: Duration::from_secs(args.timeout_secs),
            api_key: api_key(args.use_api_key)?.as_deref(),
            out_dir: args.out_dir.as_deref(),
            resume: args.resume,
            triage_model: &args.triage_model,
            cancel: Default::default(),
            progress: Some(progress),
        },
    )
    .await?;

    print!("{}", report.to_text());
    if let Some(dir) = args.prompt_out.as_deref() {
        let skipped: Vec<String> = report
            .gaps
            .iter()
            .map(|g| {
                format!(
                    "{} lane, by {} — {}",
                    g.lane,
                    g.model.as_deref().unwrap_or("nobody"),
                    g.reason
                )
            })
            .collect();
        write_prompts(
            dir,
            &repo.display().to_string(),
            &report.ranked,
            &skipped,
            report.swept.len(),
        )?;
    }

    // Proving is the expensive half, so it only runs when explicitly asked for.
    if args.prove_top > 0
        && let Some(test_command) = args.test_command.as_deref()
    {
        let proved = orchestrate::proving::prove_top(
            &report.ranked,
            &orchestrate::proving::ProveOptions {
                repo: &repo,
                model: &args.prove_model,
                test_command,
                top: args.prove_top,
                max_turns: args.max_turns,
                timeout: Duration::from_secs(args.timeout_secs),
                test_timeout: Duration::from_secs(600),
                api_key: api_key(args.use_api_key)?.as_deref(),
            },
        )
        .await;
        print!(
            "{}",
            orchestrate::proving::to_text(&proved, report.ranked.len())
        );
    }

    // A run with a hole in it must not look like a clean pass to a script.
    if !report.gaps.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}

/// Write the bundle and the per-defect prompts.
///
/// Reported on stderr rather than stdout so the report itself stays pipeable.
fn write_prompts(
    dir: &std::path::Path,
    repo: &str,
    ranked: &[bugsleuth_judge::Ranked],
    skipped: &[String],
    sweeps: usize,
) -> Result<()> {
    let written = bugsleuth_engine::handoff::write_all(dir, repo, ranked, skipped, sweeps)
        .map_err(|e| anyhow::anyhow!("cannot write prompts to {}: {e}", dir.display()))?;
    eprintln!(
        "
wrote {} and {} per-defect prompt{}",
        written.bundle.display(),
        written.per_defect_written,
        if written.per_defect_written == 1 {
            ""
        } else {
            "s"
        }
    );
    // A bundle with some per-defect files missing is an incomplete result, not a
    // success: the exit code has to say so, or a script piping this reads a
    // partial handoff as a whole one.
    if !written.warnings.is_empty() {
        anyhow::bail!(
            "the prompt bundle was saved, but some per-defect prompts failed: {}",
            written.warnings.join("; ")
        );
    }
    Ok(())
}

fn run_judge(args: JudgeArgs) -> Result<()> {
    let merged = merge::merge(&args.reports)?;
    print!("{}", merged.to_text());
    if let Some(dir) = args.prompt_out.as_deref() {
        let skipped: Vec<String> = merged
            .unswept
            .iter()
            .map(|m| format!("{} lane, by {} — {}", m.lane, m.model, m.reason))
            .collect();
        write_prompts(
            dir,
            &args.repo.display().to_string(),
            &merged.ranked,
            &skipped,
            merged.sources.len(),
        )?;
    }

    if let Some(path) = args.json_out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&merged.ranked)?;
        bugsleuth_engine::atomic::write(&path, json)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        eprintln!(
            "
wrote {}",
            path.display()
        );
    }

    // A merge that includes a failed sweep must not look like a clean pass.
    if !merged.unswept.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}

async fn run_prove(args: ProveArgs) -> Result<()> {
    let repo = real_path(&args.repo)?;

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
        bugsleuth_engine::atomic::write(&path, &report.patch)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    if !report.verdict.is_proof() {
        std::process::exit(3);
    }
    Ok(())
}

/// Canonicalize a path, then strip Windows' extended-length `\\?\` prefix.
///
/// `canonicalize` returns that prefix on Windows and most tools handle it, but
/// `git` does not: it fails with "could not create leading directories" when
/// asked to make a worktree under such a path. Stripping it costs nothing on
/// paths short enough to matter, and every path here is one a user typed.
fn real_path(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let text = canonical.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => canonical,
    })
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
    let repo = real_path(&args.repo)?;

    // The same check `run` makes, from the same function. `sweep` skipped it,
    // so a typo in --effort reached the vendor's CLI: either rejected after the
    // sweep was already paid for, or — worse — ignored, leaving a report that
    // looks normal and never says the depth asked for was not applied.
    bugsleuth_engine::plan::check_effort(&args.model, &args.effort)?;

    let api_key = api_key(args.use_api_key)?;

    let report = sweep::run(sweep::Request {
        repo: &repo,
        lane: args.lane,
        model: &args.model,
        scope: args.scope.as_deref(),
        effort: &args.effort,
        max_turns: args.max_turns,
        timeout: Duration::from_secs(args.timeout_secs),
        api_key: api_key.as_deref(),
        binary: None,
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
        bugsleuth_engine::atomic::write(&path, json)
            .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }

    // A lane that failed to run must not look like a clean pass to a script.
    if matches!(report.status, report::Status::NotSwept { .. }) {
        std::process::exit(2);
    }
    Ok(())
}
