//! Kimi Code CLI adapter.
//!
//! The fourth vendor. It exists because a Kimi *subscription* reaches models a
//! bring-your-own-key route does not: pointing an Anthropic-compatible client
//! at Moonshot's endpoint lands on whatever that endpoint defaults to, while
//! the native CLI signed in through `/login` takes `-m` and honours it.
//!
//! It shares Kilo's shape rather than Claude's, for the same reason Kilo has
//! that shape:
//!
//! **No sandbox, and approvals that cannot be loosened.** There is no
//! read-only flag, and `--yolo` and `--auto` are both *refused* alongside
//! `--prompt`. What it does have is `--agent-file`, whose `tools` frontmatter
//! is a per-invocation allowlist — that is the whole of the confinement, and
//! omitting it allows every tool. A sweep's allowlist is read-only, so a sweep
//! is additionally never pointed at the repository under review: it gets a
//! throwaway git worktree, enforced here rather than left to the caller —
//! [`KimiSweep`] takes a `worktree`, not a `repo`. An apply is granted the
//! write tools by the same mechanism and does run in the real checkout, which
//! is the one place that distinction is deliberate rather than accidental.
//!
//! **No output-schema flag.** The required JSON shape is described in the
//! brief and validated afterwards, which is strictly weaker than a schema the
//! CLI enforces. Expect a higher rate of malformed replies than from Claude or
//! Codex.
//!
//! **No stdin prompt.** `--prompt` is an argv string, which a 12 KB brief
//! cannot be — see [`brief_file`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::RawFindings;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod apply;
mod brief_file;
mod catalogue;
mod discover;
mod signin;
pub use apply::apply;
pub(crate) use catalogue::catalogue;
pub use signin::{signin, signin_for};

pub(crate) const VENDOR: &str = "kimi";

/// One read-only sweep of a throwaway checkout.
pub struct KimiSweep<'a> {
    /// A throwaway checkout the model may safely write to. **Never** the
    /// repository under review — see the module note above.
    pub worktree: &'a Path,
    /// Model alias, as Kimi's own config names it. Empty means the CLI's
    /// configured `default_model`.
    pub model: &'a str,
    /// The brief. Must already describe the required JSON shape, because this
    /// CLI cannot be given a schema to enforce.
    pub brief: &'a str,
    pub timeout: Duration,
    /// Explicit CLI path for tests; real runs use discovery.
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct KimiResult {
    pub findings: RawFindings,
}

/// The flags every Kimi invocation shares.
///
/// One list, for the reason Codex and Kilo each have one: a sign-in check or a
/// second entry point that invokes the CLI differently from the work is not
/// exercising the work.
///
/// **No approval flag.** `--yolo` and `--auto` are both *refused* alongside
/// `--prompt` — measured against the real CLI, which exits 1 with
/// `Cannot combine --prompt with --yolo`. Non-interactive mode carries its own
/// approval policy, and a real run confirms it reads repository files without
/// one. Adding either flag does not loosen anything; it stops the sweep dead.
///
/// `--output-format text` gives the assistant's own words, which is what the
/// JSON extractor reads. `stream-json` would add an event envelope to parse for
/// no gain, since the reply still has to be validated against the shape.
const BASE_FLAGS: [&str; 2] = ["--output-format", "text"];

/// Kimi reads instruction files from its working directory, exactly as the
/// others do. The worktree has already had those stripped by the engine, so
/// nothing further is needed here — but the environment is still cleared to the
/// process allowlist, so no inherited key can redirect the session.
fn sweep_environment() -> [(String, String); 0] {
    []
}

pub async fn sweep(spec: KimiSweep<'_>) -> Result<KimiResult, ProviderError> {
    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => discover::resolve_binary().ok_or_else(not_found)?,
    };
    // Written before the argv is built, and held until the invocation returns:
    // the prompt is a pointer at this file, so it has to outlive the process.
    let brief = brief_file::BriefFile::write(spec.brief, brief_file::REVIEW_AGENT)?;
    let args = build_args(&spec, &brief);
    let env = sweep_environment();

    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        // The worktree, not the repository. Kimi has no working-directory flag;
        // its workspace *is* the process working directory, so this is the one
        // thing standing between the review and the code it reviews.
        cwd: spec.worktree,
        stdin: None,
        env: &env,
        timeout: spec.timeout,
        what: "kimi CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }

    let findings = crate::json::structured(&Value::String(output.stdout.clone()))?;
    Ok(KimiResult { findings })
}

/// The argv for one sweep.
///
/// `--add-dir` grants the brief's own directory, which is deliberately outside
/// the worktree; without it the file the prompt names cannot be read. The model
/// flag is omitted entirely when empty, so the CLI's configured `default_model`
/// stands rather than being overridden with an empty string.
fn build_args(spec: &KimiSweep<'_>, brief: &brief_file::BriefFile) -> Vec<String> {
    let mut args: Vec<String> = BASE_FLAGS.iter().map(|flag| (*flag).to_string()).collect();

    args.push("--add-dir".into());
    args.push(brief.dir().to_string_lossy().into_owned());

    // Pointed at an empty directory, which is what disables discovery: the flag
    // loads skills from the given directory *instead of* the auto-discovered
    // user and project ones. A project-level skill in the reviewed tree is the
    // repository handing instructions to its own reviewer — the same hole the
    // engine strips `.kimi` and `KIMI.md` for, closed from the other side too
    // because auto-discovery also reaches outside the worktree.
    args.push("--skills-dir".into());
    args.push(brief.skills_dir().to_string_lossy().into_owned());

    // The cost and safety boundary. Without an agent definition Kimi's tool
    // allowlist is absent, which means *every* tool including `Agent` and
    // `AgentSwarm` — and K3 spontaneously delegated to subagents and burned an
    // entire billing cycle's quota inside one lane. See `brief_file`.
    args.push("--agent-file".into());
    args.push(brief.agent_path().to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }

    // Last, and its own argv entry. `--output-format` is only accepted
    // alongside `--prompt`, and the prompt is the one value here that carries
    // punctuation — passed as a single entry it is never re-parsed.
    args.push("-p".into());
    args.push(brief_file::pointer(brief.path()));
    args
}

/// Whether the CLI can be started at all, and which version answered.
///
/// The counterpart to [`signin`], and the weaker of the two: every one of these
/// CLIs starts happily while signed out. This fills the Providers panel; the
/// sign-in check is what proves a session exists.
pub async fn probe() -> Result<String, ProviderError> {
    let binary = discover::resolve_binary().ok_or_else(not_found)?;
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &["--version".to_string()],
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(60),
        what: "kimi CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    Ok(output
        .stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("unknown")
        .trim()
        .to_string())
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "install the Kimi Code CLI and sign in with `/login`".to_string(),
    }
}

#[cfg(test)]
#[path = "kimi/tests.rs"]
mod tests;
