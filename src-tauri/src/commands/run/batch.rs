//! A desktop batch owns one cancellation signal and keeps every report separate.
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use bugsleuth_engine::{cancel::Cancel, orchestrate, plan::Plan, sweep};
use serde_json::{Value, json};
use tauri::Emitter;

use super::{checked_repo, non_empty, run_output_dir};
use crate::settings::Settings;

pub(super) fn prepare(settings: &Settings) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if settings.additional_repos.len() > 15 {
        return Err("Choose at most 16 repositories per batch.".into());
    }
    let mut repositories: Vec<(PathBuf, PathBuf)> = Vec::new();
    for raw in std::iter::once(&settings.repo).chain(&settings.additional_repos) {
        let repo = checked_repo(raw)?;
        if repositories.iter().any(|(previous, _)| previous == &repo) {
            continue;
        }
        if repositories
            .iter()
            .any(|(previous, _)| previous.starts_with(&repo) || repo.starts_with(previous))
        {
            return Err("Choose separate repositories, not overlapping folders.".into());
        }
        let output = run_output_dir(&repo)?;
        repositories.push((repo, output));
    }
    Ok(repositories)
}

pub(super) async fn execute(
    app: &tauri::AppHandle,
    repositories: Vec<(PathBuf, PathBuf)>,
    plan: Plan,
    settings: Settings,
    cancel: Cancel,
) -> Value {
    let checked = tokio::select! {
        result = sweep::precheck_selected(&plan.units) => result,
        () = cancel.cancelled() => Err("Provider pre-check stopped; no lane started.".into()),
    };
    let slots = Arc::new(tokio::sync::Semaphore::new(3));
    let mut tasks = tokio::task::JoinSet::new();
    let mut identities = HashMap::new();
    let count = repositories.len();
    let mut results = vec![Value::Null; count];
    for (index, (repo, output)) in repositories.into_iter().enumerate() {
        let (app, plan, settings, cancel, slots, checked) = (
            app.clone(),
            plan.clone(),
            settings.clone(),
            cancel.clone(),
            slots.clone(),
            checked.clone(),
        );
        let identity = repo.display().to_string();
        let handle = tasks.spawn(async move {
            let _slot = slots.acquire().await;
            let result = run_one(&app, &repo, &output, &plan, &settings, &cancel, checked).await;
            (index, result)
        });
        identities.insert(handle.id(), (index, identity));
    }
    while let Some(joined) = tasks.join_next_with_id().await {
        match joined {
            Ok((id, (index, payload))) => {
                identities.remove(&id);
                results[index] = payload;
            }
            Err(error) => {
                if let Some((index, repo)) = identities.remove(&error.id()) {
                    results[index] = json!({"repo": repo, "ok": false, "complete": false,
                        "cancelled": cancel.stopped(), "text": format!("Repository task failed: {error}")});
                }
            }
        }
    }
    aggregate(results)
}

async fn run_one(
    app: &tauri::AppHandle,
    repo: &std::path::Path,
    output: &std::path::Path,
    plan: &Plan,
    settings: &Settings,
    cancel: &Cancel,
    checked: Result<(), String>,
) -> Value {
    let (progress, mut events) = tokio::sync::mpsc::unbounded_channel();
    let forwarder = app.clone();
    let identity = repo.display().to_string();
    let requested = std::iter::once(&settings.repo)
        .chain(&settings.additional_repos)
        .find(|raw| checked_repo(raw).is_ok_and(|path| path == repo))
        .cloned();
    let forwarding = tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let mut payload = json!(event);
            payload["repo"] = json!(identity);
            payload["requestedRepo"] = json!(requested);
            let _ = forwarder.emit("run-progress", payload);
        }
    });
    let report = if cancel.stopped() {
        Err(anyhow::anyhow!(
            "Review stopped before this repository started."
        ))
    } else if let Err(error) = checked {
        Err(anyhow::anyhow!(error))
    } else {
        orchestrate::run(
            plan,
            orchestrate::RunOptions {
                repo,
                scope: non_empty(&settings.scope),
                max_turns: 40,
                timeout: Duration::from_secs(2700),
                api_key: None,
                out_dir: Some(output),
                resume: settings.reuse_completed,
                triage_model: &settings.triage_model,
                cancel: cancel.clone(),
                progress: Some(progress.clone()),
            },
        )
        .await
    };
    drop(progress);
    let _ = forwarding.await;
    let cancelled = report
        .as_ref()
        .map_or(cancel.stopped(), |report| report.cancelled);
    let mut payload = crate::outcome::run_payload(report, cancelled, repo, output);
    payload["repo"] = json!(repo.display().to_string());
    if let Err(error) = super::history::save(output, &payload) {
        let previous = payload["saveError"].as_str().unwrap_or("");
        payload["saveError"] = json!(format!("{previous} {error}").trim());
    }
    payload
}

pub(super) fn aggregate(results: Vec<Value>) -> Value {
    if results.len() == 1 {
        return results.into_iter().next().unwrap_or(Value::Null);
    }
    let complete = results.iter().all(|r| r["complete"] == true);
    let cancelled = results.iter().any(|r| r["cancelled"] == true);
    // One repository stopped by a spent allowance is enough to make the batch
    // resumable, and the reason is worth carrying: it is the same allowance for
    // every repository, so the answer is to wait once rather than per folder.
    let interrupted = results
        .iter()
        .find_map(|r| r["interrupted"].as_str())
        .map(str::to_string);
    let save_error = results.iter().any(|r| !r["saveError"].is_null());
    let text = results
        .iter()
        .map(|r| {
            format!(
                "{} — {}",
                r["repo"].as_str().unwrap_or(""),
                if r["cancelled"] == true {
                    "Stopped"
                } else if r["ok"] != true {
                    "Failed"
                } else if !r["interrupted"].is_null() {
                    "Interrupted — run again to continue"
                } else if r["complete"] != true {
                    "Incomplete"
                } else if !r["saveError"].is_null() {
                    "Finished — prompt save failed"
                } else {
                    "Finished"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    json!({"ok": results.iter().any(|r| r["ok"] == true), "complete": complete,
        "cancelled": cancelled, "interrupted": interrupted, "text": text, "results": results,
        "saveError": save_error.then_some("Some repository prompts could not be saved.")})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_preserves_each_failure_and_prompt_target() {
        let results = vec![
            json!({"repo":"first", "ok":true, "complete":true, "promptPath":"first/prompt"}),
            json!({"repo":"second", "ok":false, "complete":false, "text":"provider failed"}),
        ];
        let summary = aggregate(results.clone());
        assert_eq!(summary["results"], json!(results));
        assert_eq!(summary["ok"], true);
        assert_eq!(summary["complete"], false);
        assert!(summary["promptPath"].is_null());
        assert!(
            summary["text"]
                .as_str()
                .unwrap()
                .contains("second — Failed")
        );
    }

    #[test]
    fn batch_deduplicates_canonical_paths_and_rejects_invalid_entries() {
        let mut settings = Settings {
            repo: ".".into(),
            additional_repos: vec!["./".into()],
            ..Settings::default()
        };
        assert_eq!(prepare(&settings).unwrap().len(), 1);
        settings
            .additional_repos
            .push("missing-bugsleuth-batch-repository".into());
        assert!(prepare(&settings).is_err());
        settings.additional_repos = vec![".".into(); 16];
        assert!(prepare(&settings).unwrap_err().contains("16 repositories"));
    }
}
