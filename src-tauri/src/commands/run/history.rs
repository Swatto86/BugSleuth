//! Restore the last report without running providers or changing sweep caches.
use std::path::Path;

use serde_json::{Value, json};

use crate::settings::Settings;

pub(super) fn save(output: &Path, payload: &Value) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    bugsleuth_engine::atomic::write(
        output.join("last-report.json").as_path(),
        payload.to_string(),
    )
    .map_err(|e| format!("Could not save the report for reopening: {e}"))
}

fn read(repo: &Path, output: &Path) -> Result<Option<Value>, String> {
    let path = output.join("last-report.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let mut payload: Value = serde_json::from_str(&text)
                .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            if payload["repo"] != repo.display().to_string() || !payload["text"].is_string() {
                return Err(format!("Invalid saved report in {}", path.display()));
            }
            // Never trust a persisted prompt path to choose an Apply target.
            let prompt = output.join("fix-prompt.md");
            payload["promptPath"] = if prompt.is_file() && payload["ok"] == true {
                json!(prompt.display().to_string())
            } else {
                Value::Null
            };
            Ok(Some(payload))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Older releases already saved the complete handoff, but not UI cards.
            let prompt = output.join("fix-prompt.md");
            match std::fs::read_to_string(&prompt) {
                Ok(text) => Ok(Some(json!({"repo": repo.display().to_string(),
                    "ok": true, "complete": false, "cancelled": false,
                    "text": format!("Saved handoff from an earlier scan. Coverage and source freshness have not been rechecked.\n\n{text}"),
                    "prompt": text, "promptPath": prompt.display().to_string()}))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("Cannot read {}: {e}", prompt.display())),
            }
        }
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

#[tauri::command]
pub fn load_saved_reports(settings: Settings) -> Result<Option<Value>, String> {
    if settings.repo.trim().is_empty() {
        return Ok(None);
    }
    let mut results = Vec::new();
    for (repo, output) in super::batch::prepare(&settings)? {
        if let Some(report) = read(&repo, &output)? {
            results.push(report);
        }
    }
    Ok((!results.is_empty()).then(|| super::batch::aggregate(results)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_reports_and_legacy_handoffs_without_rewriting_them() {
        let dir = std::env::temp_dir().join(format!("bugsleuth-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fix-prompt.md"), "saved findings").unwrap();
        let legacy = read(&dir, &dir).unwrap().unwrap();
        assert_eq!(legacy["prompt"], "saved findings");
        assert_eq!(legacy["complete"], false);
        let payload = json!({"repo": dir.display().to_string(), "text": "report", "ok": true,
            "complete": false, "findings": [], "promptPath": "untrusted"});
        save(&dir, &payload).unwrap();
        let restored = read(&dir, &dir).unwrap().unwrap();
        assert_eq!(restored["text"], "report");
        assert_ne!(restored["promptPath"], "untrusted");
        assert_eq!(
            std::fs::read_to_string(dir.join("fix-prompt.md")).unwrap(),
            "saved findings"
        );
        std::fs::write(dir.join("last-report.json"), "broken").unwrap();
        assert!(read(&dir, &dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
