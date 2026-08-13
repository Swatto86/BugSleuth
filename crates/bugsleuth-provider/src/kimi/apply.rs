//! Handing the fix prompt to Kimi, with write access to the real repository.
//!
//! Kimi's confinement is its agent file. `--agent-file` carries a tool
//! allowlist, and that allowlist is per invocation — the same mechanism a sweep
//! relies on to stay read-only, moved rather than removed: an apply is granted
//! `Edit`, `Write` and `Bash`, and still refused the delegation tools that once
//! burned a whole billing cycle inside one lane. See
//! [`brief_file::APPLY_AGENT`].
//!
//! What that allowlist does *not* do is bound where a write may land. A sweep
//! answers that by never being pointed at the repository under review; an apply
//! cannot, because editing the real checkout is the entire point. So beyond the
//! allowlist the safety story is git: the engine refuses to start unless the
//! working tree is clean, so everything this does shows up in `git status` and
//! can be thrown away with one command.

use std::path::Path;
use std::time::Duration;

use crate::error::ProviderError;
use crate::process::{self, Invocation, preview};

use super::{BASE_FLAGS, VENDOR, brief_file, discover, not_found};

/// Apply the fixes described in `prompt`, returning the model's own account.
pub async fn apply(
    repo: &Path,
    model: &str,
    _effort: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, ProviderError> {
    let binary = discover::resolve_binary().ok_or_else(not_found)?;

    // Written before the argv is built and held until the invocation returns:
    // the prompt is a pointer at this file, so it has to outlive the process.
    // The agent definition beside it is what the CLI is confined by.
    let handoff = brief_file::BriefFile::write(prompt, brief_file::APPLY_AGENT)?;
    let args = build_args(&handoff, model);

    let output = process::run(Invocation {
        binary: &binary.to_string_lossy(),
        args: &args,
        // Kimi has no working-directory flag; its workspace *is* the process
        // working directory. For an apply that is the repository itself.
        cwd: repo,
        stdin: None,
        env: &[],
        timeout,
        what: "kimi CLI",
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

/// The argv for one write-capable invocation.
///
/// `--effort` has no counterpart on this CLI, so the caller's value is not
/// silently turned into something else.
fn build_args(handoff: &brief_file::BriefFile, model: &str) -> Vec<String> {
    let mut args: Vec<String> = BASE_FLAGS.iter().map(|flag| (*flag).to_string()).collect();

    // The handoff lives outside the repository — a fix prompt sitting inside
    // the tree being fixed is one more file the model can trip over — so the
    // session has to be granted that directory to read it at all.
    args.push("--add-dir".into());
    args.push(handoff.dir().to_string_lossy().into_owned());

    // An empty directory, pointed at so discovery is *replaced* rather than
    // added to: neither the user's own skills nor any the repository ships are
    // loaded into a session that can write.
    args.push("--skills-dir".into());
    args.push(handoff.skills_dir().to_string_lossy().into_owned());

    // The boundary. Without it Kimi's allowlist is absent, which means every
    // tool including the ones that spawn subagents.
    args.push("--agent-file".into());
    args.push(handoff.agent_path().to_string_lossy().into_owned());

    let model = model.trim();
    if !model.is_empty() {
        args.push("-m".into());
        args.push(model.to_string());
    }

    // Last, and its own argv entry, for the reason a sweep does the same: the
    // prompt is the one value carrying punctuation, and as a single entry it is
    // never re-parsed.
    args.push("-p".into());
    args.push(pointer(handoff.path()));
    args
}

/// What Kimi is actually told, which is where to find the rest.
///
/// Short and free of quotes and newlines beyond the path: this is the one
/// string that still crosses the command line.
fn pointer(handoff: &Path) -> String {
    format!(
        "Read the file at {} and carry out the fixes it describes, exactly as written. Change \
         only files inside the repository you are running in, and end by reporting what you \
         actually changed.",
        handoff.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: [&str; 6] = ["Read", "Grep", "Glob", "Edit", "Write", "Bash"];
    const DENIED: [&str; 3] = ["Agent", "AgentSwarm", "Skill"];

    fn list_after<'a>(definition: &'a str, heading: &str) -> Vec<&'a str> {
        definition
            .lines()
            .skip_while(|line| line.trim() != heading)
            .skip(1)
            .take_while(|line| line.trim_start().starts_with("- "))
            .map(|line| line.trim().trim_start_matches("- "))
            .collect()
    }

    fn argv(model: &str) -> (brief_file::BriefFile, Vec<String>) {
        let handoff = brief_file::BriefFile::write("fix the finding", brief_file::APPLY_AGENT)
            .expect("write handoff");
        let args = build_args(&handoff, model);
        (handoff, args)
    }

    #[test]
    fn the_run_is_confined_by_an_agent_file_that_allows_writing() {
        let (handoff, args) = argv("kimi-k3");
        let agent = args
            .iter()
            .position(|a| a == "--agent-file")
            .and_then(|i| args.get(i + 1))
            .expect("the run names an agent file");
        assert_eq!(agent, &handoff.agent_path().to_string_lossy().into_owned());

        let definition = std::fs::read_to_string(agent).expect("the agent file exists");
        assert_eq!(
            list_after(&definition, "tools:"),
            ALLOWED,
            "unexpected apply allowlist: {definition}"
        );
        assert_eq!(
            list_after(&definition, "disallowedTools:"),
            DENIED,
            "unexpected apply denylist: {definition}"
        );
    }

    #[test]
    fn agent_tool_lists_keep_allowed_and_denied_entries_separate() {
        let malformed = r#"tools:
  - Read
  - Grep
  - Glob
  - Edit
  - Write
  - Bash
  - Agent
disallowedTools:
  - AgentSwarm
  - Skill"#;
        assert_ne!(list_after(malformed, "tools:"), ALLOWED);
        assert_ne!(list_after(malformed, "disallowedTools:"), DENIED);
    }

    #[test]
    fn skills_are_replaced_by_an_empty_directory_rather_than_left_discovered() {
        let (handoff, args) = argv("");
        let dir = args
            .iter()
            .position(|a| a == "--skills-dir")
            .and_then(|i| args.get(i + 1))
            .expect("the run names a skills directory");
        assert_eq!(dir, &handoff.skills_dir().to_string_lossy().into_owned());
        assert_eq!(
            std::fs::read_dir(dir)
                .expect("skills directory exists")
                .count(),
            0,
            "the skills directory is not empty"
        );
    }

    #[test]
    fn the_prompt_that_crosses_the_command_line_stays_small() {
        // The handoff for a real run is thousands of characters; `cmd.exe` caps
        // a command line at 8,191 and an npm-shim install goes through it.
        let (handoff, args) = argv("");
        let prompt = args.last().expect("the prompt is last");
        assert!(prompt.len() < 1_024, "the prompt is {} chars", prompt.len());
        assert!(
            prompt.contains(&handoff.path().display().to_string()),
            "the prompt does not point at the handoff: {prompt}"
        );
    }

    #[test]
    fn an_empty_model_is_omitted_rather_than_passed_blank() {
        let (_handoff, args) = argv("   ");
        assert!(!args.iter().any(|a| a == "-m"), "{args:?}");
    }

    #[tokio::test]
    async fn a_missing_cli_is_reported_as_not_installed_rather_than_as_a_failed_apply() {
        if discover::resolve_binary().is_some() {
            return;
        }
        let error = apply(
            Path::new("."),
            "",
            "",
            "fix the finding",
            Duration::from_secs(5),
        )
        .await
        .expect_err("no CLI means no apply");
        assert!(error.to_string().contains("kimi"), "{error}");
    }
}
