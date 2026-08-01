//! Kilo CLI adapter.
//!
//! The third vendor, and the awkward one. Two things it does not have that the
//! others do, both of which change the design rather than just the argv:
//!
//! **No output-schema flag.** Claude takes a JSON Schema inline and Codex takes
//! one as a file; Kilo takes neither. The schema therefore has to be described
//! in the prompt and the reply validated afterwards, which is strictly weaker —
//! expect a higher rate of malformed replies from this vendor than the others.
//!
//! **No read-only mode.** Codex has `--sandbox read-only` and Claude has a tool
//! allowlist. Kilo's permissions come from the *user's own global config*, and
//! on the machine this was written against both candidate agents (`ask` and
//! `plan`) were configured to allow everything. There is no per-invocation flag
//! that overrides it.
//!
//! So a Kilo sweep is never pointed at the repository under review. It is given
//! a throwaway git worktree, which the caller deletes afterwards. That is
//! enforced here rather than left to the caller: [`KiloSweep`] takes a
//! `worktree`, not a `repo`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bugsleuth_domain::RawFindings;
use serde_json::Value;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

mod discover;
mod events;

pub(crate) const VENDOR: &str = "kilo";

/// Which account a Kilo model spends from.
///
/// Kilo encodes this in the model id's first segment, and it matters more than
/// it looks: the same underlying model can be reached three different ways, and
/// they bill to three different places. `kilo/z-ai/glm-5` spends Kilo Gateway
/// credit; `openrouter/z-ai/glm-5` spends your own OpenRouter key. Nothing in
/// the run output would otherwise tell you which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Kilo Gateway — the subscription/Kilo Pass route.
    Gateway,
    /// Reached *through* Kilo but billed to your own plan with that vendor —
    /// the coding-plan integrations, like Kimi Code and the Z.ai Coding Plan.
    ///
    /// These carry a `kilo/` prefix like any gateway model, so the id alone
    /// cannot tell them apart. Only the catalogue's own `hasUserByokAvailable`
    /// flag does.
    KiloByok,
    /// Your own OpenRouter API key. Bring-your-own-key.
    OpenRouter,
    /// A signed-in OpenAI account.
    OpenAi,
    /// A model running locally.
    Ollama,
    /// No prefix, so Kilo picks from its own configuration.
    Configured,
}

impl Route {
    pub fn describe(self) -> &'static str {
        match self {
            Route::Gateway => "Kilo Gateway (subscription)",
            Route::KiloByok => "Your own plan via Kilo (BYOK)",
            Route::OpenRouter => "OpenRouter (your own API key)",
            Route::OpenAi => "OpenAI account",
            Route::Ollama => "local via Ollama",
            Route::Configured => "Kilo's configured default",
        }
    }
}

/// Where the Kilo CLI lives, if it is installed.
///
/// Exposed so the model catalogue can be fetched without `models.rs` having to
/// know how this vendor is discovered.
#[must_use]
pub fn binary_path() -> Option<std::path::PathBuf> {
    discover::resolve_binary()
}

/// Read the billing route out of a Kilo model id.
pub fn route_of(model: &str) -> Route {
    match model.trim().split('/').next().unwrap_or_default() {
        "kilo" => Route::Gateway,
        "openrouter" => Route::OpenRouter,
        "openai" => Route::OpenAi,
        "ollama" => Route::Ollama,
        _ => Route::Configured,
    }
}

pub struct KiloSweep<'a> {
    /// A throwaway checkout the model may safely write to. **Never** the
    /// repository under review — see the module note above.
    pub worktree: &'a Path,
    /// Model in Kilo's `provider/model` form. Empty means its configured default.
    pub model: &'a str,
    /// Reasoning effort. Kilo calls this a model *variant* and passes it
    /// straight through to the provider, so the accepted values depend on the
    /// model rather than on Kilo. Empty means whatever the model does by default.
    pub effort: &'a str,
    /// The brief. Must already describe the required JSON shape, because this
    /// CLI cannot be given a schema to enforce.
    pub brief: &'a str,
    pub timeout: Duration,
    pub binary: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct KiloResult {
    pub findings: RawFindings,
}

pub async fn sweep(spec: KiloSweep<'_>) -> Result<KiloResult, ProviderError> {
    let binary = match spec.binary {
        Some(path) => PathBuf::from(path),
        None => discover::resolve_binary().ok_or_else(not_found)?,
    };

    let args = build_args(&spec);
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        cwd: spec.worktree,
        stdin: Some(spec.brief.as_bytes()),
        env: &[],
        timeout: spec.timeout,
        what: "kilo CLI",
    })
    .await?;

    if !output.succeeded() {
        let code = output.code.unwrap_or(-1);
        // stderr *then* stdout. Kilo says almost nothing on stderr and streams
        // its errors as NDJSON events on stdout alongside everything else, so
        // looking only at stderr reported "no diagnostic output" for a failure
        // that had said exactly what was wrong. That cost a long time to find.
        let message = match preview(output.stderr.trim(), 2000) {
            text if !text.is_empty() => text,
            _ => events::error_events(&output.stdout),
        };
        return Err(if message.is_empty() {
            ProviderError::FailedSilently {
                vendor: VENDOR,
                code,
            }
        } else {
            ProviderError::Failed {
                vendor: VENDOR,
                code,
                message,
            }
        });
    }

    let text = events::assistant_text(&output.stdout);
    if text.trim().is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    Ok(KiloResult {
        findings: crate::json::structured(&Value::String(text))?,
    })
}

/// Build the non-interactive argv.
///
/// `--pure` is Kilo's nearest equivalent to the other vendors' safe modes: it
/// skips external plugins, so a plugin installed on this machine cannot change
/// what the review does. It does **not** neutralise agent permissions, which is
/// why the worktree exists.
fn build_args(spec: &KiloSweep<'_>) -> Vec<String> {
    let mut args: Vec<String> = [
        "run", "--auto", "--pure", "--format", "json", "--agent", "ask",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    // Pin the working directory explicitly as well as via the spawned process's
    // cwd. Kilo resolves some paths from `--dir` rather than the process cwd,
    // and a mismatch between the two would have it review a different tree than
    // the one whose anchors we later verify against.
    args.push("--dir".into());
    args.push(spec.worktree.to_string_lossy().into_owned());

    let model = spec.model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }
    let effort = spec.effort.trim();
    if !effort.is_empty() {
        args.push("--variant".into());
        args.push(effort.to_string());
    }
    args
}

fn not_found() -> ProviderError {
    ProviderError::NotFound {
        vendor: VENDOR,
        hint: "Install it with `npm install -g @kilocode/cli` and sign in with `kilo auth`, or \
               pass an explicit binary path."
            .to_string(),
    }
}

/// Check the CLI exists and can run. Free — starts no model.
pub async fn probe() -> Result<String, ProviderError> {
    let binary = discover::resolve_binary().ok_or_else(not_found)?;
    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &["--version".to_string()],
        cwd: Path::new("."),
        stdin: None,
        env: &[],
        timeout: Duration::from_secs(60),
        what: "kilo CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 500),
        });
    }
    // Kilo prints a banner before the version; keep the last non-empty line.
    let version = output
        .stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("unknown")
        .trim()
        .to_string();
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(model: &'a str) -> KiloSweep<'a> {
        KiloSweep {
            effort: "",
            worktree: Path::new("/tmp/wt"),
            model,
            brief: "",
            timeout: Duration::from_secs(60),
            binary: None,
        }
    }

    #[test]
    fn the_model_id_says_which_account_a_sweep_will_spend_from() {
        // The same model reached three ways bills to three different places.
        assert_eq!(route_of("kilo/z-ai/glm-5"), Route::Gateway);
        assert_eq!(route_of("openrouter/z-ai/glm-5"), Route::OpenRouter);
        assert_eq!(route_of("openai/gpt-5"), Route::OpenAi);
        assert_eq!(route_of("ollama/llama3"), Route::Ollama);
    }

    #[test]
    fn an_id_with_no_prefix_leaves_the_choice_to_kilo() {
        assert_eq!(route_of(""), Route::Configured);
        assert_eq!(route_of("  "), Route::Configured);
        assert_eq!(route_of("some-model"), Route::Configured);
    }

    #[test]
    fn every_route_can_be_described_to_a_person() {
        for route in [
            Route::Gateway,
            Route::OpenRouter,
            Route::OpenAi,
            Route::Ollama,
            Route::Configured,
        ] {
            assert!(!route.describe().is_empty());
        }
    }

    #[test]
    fn external_plugins_are_skipped_so_the_machine_cannot_change_the_review() {
        assert!(build_args(&spec("")).iter().any(|a| a == "--pure"));
    }

    #[test]
    fn the_working_directory_is_pinned_explicitly() {
        let args = build_args(&spec(""));
        let dir = args
            .iter()
            .position(|a| a == "--dir")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str);
        assert_eq!(dir, Some("/tmp/wt"));
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_as_a_blank_argument() {
        assert!(!build_args(&spec("   ")).iter().any(|a| a == "-m"));
    }
}
