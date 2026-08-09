//! Reading Kilo's NDJSON event stream.
//!
//! `--format json` makes Kilo emit one JSON event per line for everything it
//! does — including its failures. Both functions here exist because of a real
//! bug each.

use serde_json::Value;

use crate::process::preview;

/// The conversation id carried by Kilo events, if the run got far enough to
/// create one. Read from any event because current releases repeat it on all of
/// them, including the first `step_start` written before a timed-out run.
pub(super) fn session_id(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let event: Value = serde_json::from_str(line).ok()?;
        event["sessionID"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
    })
}

/// Any error Kilo reported in its event stream, as one line.
///
/// The events look like
/// `{"type":"error","error":{"name":"...","data":{"message":"..."}}}`.
/// Read one line at a time rather than deserializing the whole stream: this runs
/// on a failure path where the output may well be truncated or malformed, and a
/// parser that gives up on the first bad line would throw away the diagnosis.
pub(super) fn error_events(stdout: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value["type"].as_str() != Some("error") {
            continue;
        }
        let error = &value["error"];
        let name = error["name"].as_str().unwrap_or("error");
        let detail = error["data"]["message"]
            .as_str()
            .or_else(|| error["message"].as_str())
            .unwrap_or_default();
        let text = if detail.is_empty() {
            name.to_string()
        } else {
            format!("{name}: {detail}")
        };
        if !seen.contains(&text) {
            seen.push(text);
        }
    }
    preview(&seen.join("; "), 2000)
}

/// The assistant's final message, from Kilo's NDJSON event stream.
///
/// **Not a concatenation.** Each `text` event carries the *complete* text of its
/// message, not an incremental delta, and Kilo emits the same message more than
/// once. Appending them produced `{"findings":[]}{"findings":[]}` — two valid
/// objects glued into one invalid document — and the first real Kilo sweep
/// failed on exactly that.
///
/// So events are grouped by message id, the latest text wins within a message,
/// and the last message is the answer. Earlier messages are the agent narrating
/// its progress ("I'll start by exploring the repository structure"), which is
/// not the reply and must not be mixed into it.
///
/// The payload is nested under a `part` object in current versions, with an
/// older flat shape still in the wild. Both are read, and unparseable lines are
/// skipped rather than failing the sweep: the wire format is still moving
/// upstream and one unknown event should not discard a completed review.
pub(super) fn assistant_text(stdout: &str) -> String {
    // (message id, latest text) in first-seen order. A handful of messages, so
    // a linear scan is cheaper than a map and keeps the ordering for free.
    let mut messages: Vec<(String, String)> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event["type"].as_str() != Some("text") {
            continue;
        }
        let Some(text) = event["part"]["text"]
            .as_str()
            .or_else(|| event["text"].as_str())
        else {
            continue;
        };
        // Messages without an id are treated as one anonymous message rather
        // than being dropped; older event shapes carry no id at all.
        let id = event["part"]["messageID"]
            .as_str()
            .or_else(|| event["messageID"].as_str())
            .unwrap_or("")
            .to_string();

        match messages.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, existing)) => *existing = text.to_string(),
            None => messages.push((id, text.to_string())),
        }
    }

    messages.pop().map(|(_, text)| text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_events_accepts_legal_json_whitespace() {
        let stdout =
            r#"{"type": "error", "error": {"name": "AuthError", "data": {"message": "sign in"}}}"#;
        assert_eq!(error_events(stdout), "AuthError: sign in");
    }

    #[test]
    fn a_session_id_survives_noise_and_truncated_events() {
        let stdout = "not json\n{\"type\":\"step_start\",\"sessionID\":\"abc\"}\n{";
        assert_eq!(session_id(stdout).as_deref(), Some("abc"));
        assert_eq!(session_id(r#"{"sessionID":""}"#), None);
    }

    #[test]
    fn a_message_repeated_by_the_stream_is_not_glued_to_itself() {
        // Kilo really does emit the same complete message twice. Concatenating
        // these produced `{"findings":[]}{"findings":[]}` and the first real
        // Kilo sweep failed to parse.
        let stdout = concat!(
            r#"{"type":"step_start"}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m1","text":"{\"findings\":[]}"}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m1","text":"{\"findings\":[]}"}}"#,
            "\n",
            r#"{"type":"step_finish"}"#,
        );
        assert_eq!(assistant_text(stdout), r#"{"findings":[]}"#);
    }

    #[test]
    fn narration_from_earlier_messages_is_not_mixed_into_the_answer() {
        let stdout = concat!(
            r#"{"type":"text","part":{"messageID":"m1","text":"I'll start by exploring."}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m2","text":"{\"findings\":[]}"}}"#,
        );
        assert_eq!(assistant_text(stdout), r#"{"findings":[]}"#);
    }

    #[test]
    fn the_older_flat_event_shape_is_still_read() {
        let stdout = r#"{"type":"text","text":"hello"}"#;
        assert_eq!(assistant_text(stdout), "hello");
    }

    #[test]
    fn a_stream_with_no_text_events_yields_nothing_rather_than_panicking() {
        assert_eq!(assistant_text(r#"{"type":"step_finish"}"#), "");
        assert_eq!(assistant_text(""), "");
    }

    #[test]
    fn a_malformed_event_is_skipped_rather_than_discarding_the_whole_review() {
        let stdout = concat!(
            "not json at all\n",
            r#"{"type":"text","part":{"text":"kept"}}"#,
            "\n{ broken json\n",
        );
        assert_eq!(assistant_text(stdout), "kept");
    }
}

#[cfg(test)]
mod signin_extraction_tests {
    use super::*;

    /// Event noise alone is not an answer.
    ///
    /// Kilo prints NDJSON, so a sign-in check asking only "was stdout empty"
    /// would call a session working on the strength of its own logging. The
    /// probe pulls the assistant text out first, so a run that logged plenty
    /// and said nothing is reported as "cannot tell" rather than as a pass.
    #[test]
    fn a_run_that_logged_but_never_answered_yields_no_text() {
        let noise = concat!(
            r#"{"type":"step_start","part":{"messageID":"m1"}}"#,
            "\n",
            r#"{"type":"step_finish","part":{"messageID":"m1","reason":"stop"}}"#,
            "\n"
        );
        assert!(
            assistant_text(noise).trim().is_empty(),
            "event noise was read as an answer"
        );
    }

    /// And a real answer survives, so the check is not vacuous either way.
    #[test]
    fn the_models_own_words_come_back() {
        let answered = concat!(
            r#"{"type":"step_start","part":{"messageID":"m1"}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m1","text":"OK"}}"#,
            "\n"
        );
        assert_eq!(assistant_text(answered).trim(), "OK");
    }
}
