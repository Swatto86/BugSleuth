//! The M0 harness: run one lane against one repository with one model, verify
//! every anchor, and print what survived.

mod cli;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use bugsleuth_engine::{merge, orchestrate, plan, report, sweep};
use clap::Parser;

use cli::{Cli, Command, JudgeArgs, RunArgs, SweepArgs};

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Sweep(args) => run_sweep(args).await,
        Command::Run(args) => run_all(args).await,
        Command::Judge(args) => run_judge(args),
        Command::Preflight => sweep::preflight().await,
    }
}

async fn run_all(args: RunArgs) -> Result<()> {
    let plan = plan::load(&args.config)?;
    let uses_claude = plan
        .units
        .iter()
        .any(|unit| matches!(sweep::Vendor::parse(&unit.model).0, sweep::Vendor::Claude));
    let max_turns = claude_turn_budget(args.max_turns, uses_claude, 40)?;
    // Triage is a Claude call, so a plan of Codex and Kilo units with a triage
    // model configured still has somewhere to spend the key.
    validate_api_key_target(
        args.use_api_key,
        uses_claude || !args.triage_model.trim().is_empty(),
    )?;
    let repo = real_path(&args.repo)?;

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
            max_turns,
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
    // A UNC network path must keep its `\\server\share` form rather than be
    // truncated to a relative `UNC\server\share`; one shared conversion.
    Ok(bugsleuth_engine::git_path(&canonical))
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

/// Refuse API-key mode for a provider the key cannot reach.
///
/// `--use-api-key` supplies `ANTHROPIC_API_KEY`, and only the Claude adapter
/// takes one — Codex and Kilo use the session configured in their own CLI. The
/// flag was accepted for every model, so a Codex or Kilo sweep read the key and
/// then ran without it: the signed-in account the user had explicitly asked not
/// to use was used, and charged, with nothing said.
///
/// A mixed `run` is allowed, because the key really does apply to its Claude
/// units and to the Claude triage pass; only a run that reaches no Claude call
/// at all is refused.
fn validate_api_key_target(requested: bool, uses_claude: bool) -> Result<()> {
    if requested && !uses_claude {
        anyhow::bail!(
            "--use-api-key supplies ANTHROPIC_API_KEY and is only supported by Claude; Codex \
             and Kilo use the session configured in their own CLI"
        );
    }
    Ok(())
}

fn claude_turn_budget(requested: Option<u32>, uses_claude: bool, default: u32) -> Result<u32> {
    if requested.is_some() && !uses_claude {
        anyhow::bail!(
            "--max-turns is only supported by Claude; Codex and Kilo use a per-invocation \
             --timeout-secs limit plus one bounded recovery"
        );
    }
    Ok(requested.unwrap_or(default))
}

fn sweep_start_message(
    repo: &Path,
    lane: bugsleuth_domain::Lane,
    model: &str,
    timeout_secs: u64,
) -> String {
    format!(
        "Starting {lane} sweep of {} with {model} (timeout {timeout_secs}s)…",
        repo.display()
    )
}

async fn run_sweep(args: SweepArgs) -> Result<()> {
    let uses_claude = matches!(sweep::Vendor::parse(&args.model).0, sweep::Vendor::Claude);
    let max_turns = claude_turn_budget(args.max_turns, uses_claude, 30)?;
    // Before `api_key` reads the environment, so the refusal names the real
    // problem rather than complaining that an irrelevant variable is unset.
    validate_api_key_target(args.use_api_key, uses_claude)?;
    let repo = real_path(&args.repo)?;

    // The same check `run` makes, from the same function. `sweep` skipped it,
    // so a typo in --effort reached the vendor's CLI: either rejected after the
    // sweep was already paid for, or — worse — ignored, leaving a report that
    // looks normal and never says the depth asked for was not applied.
    bugsleuth_engine::plan::check_effort(&args.model, &args.effort)?;

    let api_key = api_key(args.use_api_key)?;

    eprintln!(
        "{}",
        sweep_start_message(&repo, args.lane, &args.model, args.timeout_secs)
    );
    let report = sweep::run(sweep::Request {
        repo: &repo,
        lane: args.lane,
        model: &args.model,
        scope: args.scope.as_deref(),
        effort: &args.effort,
        max_turns,
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

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    // No `use super::*`: only the Windows-only block below reaches into `super`
    // (for `real_path`), so importing it unconditionally is an unused import on
    // other platforms — which fails a `-D warnings` build.

    #[test]
    fn a_single_sweep_announces_itself_before_waiting() {
        assert_eq!(
            super::sweep_start_message(
                std::path::Path::new("repo"),
                bugsleuth_domain::Lane::Ux,
                "haiku",
                7,
            ),
            "Starting UX sweep of repo with haiku (timeout 7s)…"
        );
    }

    #[tokio::test]
    async fn a_non_claude_sweep_rejects_an_explicit_turn_budget_before_repo_work() {
        let cli = super::Cli::try_parse_from([
            "bugsleuth",
            "sweep",
            "--repo",
            "missing-turn-budget-test-repo",
            "--lane",
            "security",
            "--model",
            "codex:",
            "--max-turns",
            "1",
        ])
        .expect("arguments parse");
        let super::Command::Sweep(args) = cli.command else {
            panic!("sweep command parsed as another command");
        };

        let error = super::run_sweep(args)
            .await
            .expect_err("budget is rejected");
        assert!(
            error.to_string().contains("only supported by Claude"),
            "{error}"
        );
    }

    #[test]
    fn a_claude_turn_budget_defaults_and_applies_when_claude_is_present() {
        assert_eq!(super::claude_turn_budget(None, false, 30).unwrap(), 30);
        assert_eq!(super::claude_turn_budget(Some(7), true, 40).unwrap(), 7);
        assert!(super::claude_turn_budget(Some(1), false, 30).is_err());
    }

    #[test]
    fn api_key_mode_is_refused_for_a_provider_it_cannot_reach() {
        // `--use-api-key` supplies ANTHROPIC_API_KEY. Accepting it for a Codex
        // or Kilo sweep read the key and then invoked a CLI that never receives
        // it, so the signed-in account the user meant to avoid was used and
        // charged anyway.
        assert!(super::validate_api_key_target(false, false).is_ok());
        assert!(super::validate_api_key_target(true, true).is_ok());
        let error =
            super::validate_api_key_target(true, false).expect_err("api key mode is rejected");
        assert!(
            error.to_string().contains("only supported by Claude"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_non_claude_sweep_rejects_api_key_mode_before_reading_the_environment() {
        // Ordering matters, and this holds whichever way the environment is set
        // up: without the fix an unset key complains that it is missing and a
        // set one gets as far as the missing repository. Neither says this.
        let cli = super::Cli::try_parse_from([
            "bugsleuth",
            "sweep",
            "--repo",
            "missing-api-key-test-repo",
            "--lane",
            "security",
            "--model",
            "codex:",
            "--use-api-key",
        ])
        .expect("arguments parse");
        let super::Command::Sweep(args) = cli.command else {
            panic!("sweep command parsed as another command");
        };

        let error = super::run_sweep(args)
            .await
            .expect_err("api key mode is rejected");
        assert!(
            error.to_string().contains("only supported by Claude"),
            "the environment was consulted before the provider was checked: {error}"
        );
    }

    #[tokio::test]
    async fn a_non_claude_run_rejects_an_explicit_turn_budget_before_repo_work() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_nanos();
        let config = std::env::temp_dir().join(format!(
            "bugsleuth-turn-budget-{}-{unique}.json",
            std::process::id()
        ));
        std::fs::write(
            &config,
            r#"{"models":[{"id":"kilo:","lanes":["security"]}]}"#,
        )
        .expect("write config");
        let repo = config.with_extension("missing-repo");
        assert!(!repo.exists(), "the missing-repo control exists");

        let result = super::run_all(super::RunArgs {
            repo,
            config: config.clone(),
            scope: None,
            max_turns: Some(1),
            timeout_secs: 1,
            out_dir: None,
            resume: false,
            use_api_key: false,
            triage_model: String::new(),
            prompt_out: None,
        })
        .await;
        std::fs::remove_file(config).expect("remove config");

        let error = result.expect_err("budget is rejected");
        assert!(
            error.to_string().contains("only supported by Claude"),
            "{error}"
        );
    }

    #[test]
    fn unc_verbatim_path() {
        // The shared conversion `real_path` uses: a UNC network path must
        // survive as `\\server\share`, not the relative `UNC\server\share` that
        // dropping only `\\?\` leaves. Pure string work, so run everywhere.
        assert_eq!(
            bugsleuth_engine::git_path(std::path::Path::new(r"\\?\UNC\server\share\repo"))
                .to_string_lossy(),
            r"\\server\share\repo"
        );
        // End-to-end on this platform: real_path must never hand back a path
        // still wearing the extended-length prefix git rejects.
        #[cfg(windows)]
        {
            let resolved = super::real_path(std::path::Path::new(".")).expect("the cwd resolves");
            assert!(
                !resolved.to_string_lossy().starts_with(r"\\?\"),
                "real_path returned an extended-length path: {}",
                resolved.display()
            );
        }
    }
}
