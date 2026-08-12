//! Handing the fix prompt to Cursor Agent with write access to the real repository.
//!
//! A sweep uses `--mode ask` (read-only). An apply drops that flag and passes
//! `--force` so print mode actually edits files. The working tree must already
//! be clean — the engine refuses otherwise — so everything this does shows up
//! in `git status`.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

use super::{VENDOR, brief_file, discover, not_found};

/// Cursor-relevant instruction names, lowercase. Matched case-insensitively
/// anywhere under the repository — the same rule sweeps use when stripping.
const INSTRUCTION_FILES: &[&str] = &[".cursorrules", "agents.md", "cursor.md"];
const INSTRUCTION_DIRS: &[&str] = &[".cursor", ".agents"];
const SKIP: &[&str] = &[".git", "target", "node_modules", "dist", "build", "vendor"];

/// Why this repository cannot be applied into with Cursor, or `None` when it
/// does not ship Cursor instruction files.
fn repository_instructions_present(repo: &Path) -> Option<String> {
    let found = find_instruction(repo, repo)?;
    Some(format!(
        "{found} in this repository would brief the Cursor agent that applies \
         fixes, so the repository would be choosing its own execution policy. \
         Apply the generated handoff manually, or move that configuration out \
         of the repository first."
    ))
}

fn find_instruction(dir: &Path, root: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let is_link = metadata.file_type().is_symlink() || is_reparse_point(&metadata);

        if INSTRUCTION_DIRS.contains(&name.as_str()) {
            return Some(relative(&path, root));
        }
        if is_link {
            // Do not walk through outbound links or cycles; an instruction-
            // named link already returned above.
            continue;
        }
        if metadata.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            if let Some(found) = find_instruction(&path, root) {
                return Some(found);
            }
        } else if INSTRUCTION_FILES.contains(&name.as_str()) {
            return Some(relative(&path, root));
        }
    }
    None
}

/// Same mask as bugsleuth-verify's orphan cleanup: junctions are reparse
/// points and must not be treated as ordinary directories to walk into.
#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    repo: &Path,
    model: &str,
    _effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    if let Some(reason) = repository_instructions_present(repo) {
        return Err(ProviderError::CapabilityUnavailable {
            vendor: VENDOR,
            capability: "apply",
            reason,
        });
    }

    let launch = match discover::resolve() {
        Some(launch) => launch,
        None => return Err(not_found()),
    };

    // Written into the repository itself: Cursor has no grant for reading a
    // path outside `--workspace`, and the workspace *is* the repository for an
    // apply. Removed on drop so a finished apply does not leave the handoff
    // behind; a killed apply may leave it, which `git status` will show.
    let handoff = brief_file::BriefFile::write_in(repo, prompt)?;
    let args = build_args(&launch.prefix, handoff.path(), model);

    let output = process::run(Invocation {
        binary: &launch.binary.to_string_lossy(),
        args: &args,
        cwd: repo,
        stdin: None,
        env: &[],
        timeout,
        what: "cursor agent CLI",
    })
    .await?;

    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: preview(output.stderr.trim(), 2000),
        });
    }

    let report = output.stdout.trim();
    if report.is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    Ok(report.to_string())
}

fn build_args(prefix: &[String], handoff: &Path, model: &str) -> Vec<String> {
    let mut args = prefix.to_vec();
    args.extend([
        "-p".into(),
        "--force".into(),
        "--trust".into(),
        "--output-format".into(),
        "text".into(),
    ]);

    let model = model.trim();
    if !model.is_empty() {
        args.push("--model".into());
        args.push(model.to_string());
    }

    // Relative name so the prompt stays short; the handoff sits in the repo root.
    let name = handoff
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| brief_file::BRIEF_NAME.to_string());
    args.push(format!(
        "Read ./{name} and carry out the fixes it describes, exactly as written. Change \
         only files inside this workspace, and end by reporting what you actually changed."
    ));
    args
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
