//! OpenCode reports failures in JSON events even with exit code zero.
use super::{ProviderError, VENDOR, process};
use serde_json::Value;

pub(super) fn answer(output: process::CliOutput) -> Result<String, ProviderError> {
    let mut messages: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in output.stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event["type"] == "error" {
            return Err(ProviderError::Failed {
                vendor: VENDOR,
                code: output.code.unwrap_or(-1),
                message: process::redact_secrets(&process::preview(
                    &event["error"].to_string(),
                    2000,
                )),
            });
        }
        if event["type"] != "text" {
            continue;
        }
        let part = &event["part"];
        let Some(text) = part["text"].as_str().or_else(|| event["text"].as_str()) else {
            continue;
        };
        let message = part["messageID"].as_str().unwrap_or("");
        let id = part["id"].as_str().unwrap_or("");
        if !messages.iter().any(|(m, _)| m == message) {
            messages.push((message.into(), Vec::new()));
        }
        if let Some((_, parts)) = messages.iter_mut().find(|(m, _)| m == message) {
            if let Some((_, value)) = parts.iter_mut().find(|(p, _)| p == id) {
                *value = text.into();
            } else {
                parts.push((id.into(), text.into()));
            }
        }
    }
    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: VENDOR,
            code: output.code.unwrap_or(-1),
            message: process::redact_secrets(&process::preview(output.stderr.trim(), 2000)),
        });
    }
    let answer: String = messages
        .pop()
        .map(|(_, parts)| parts.into_iter().map(|(_, text)| text).collect())
        .unwrap_or_default();
    if answer.trim().is_empty() {
        return Err(ProviderError::Empty(VENDOR));
    }
    Ok(answer)
}
