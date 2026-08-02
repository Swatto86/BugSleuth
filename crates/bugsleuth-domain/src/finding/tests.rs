//! Tests for findings and the sweep envelope, in their own file only
//! because the module plus its tests crossed the hard line cap.

use super::*;

/// Build a minimal value satisfying a schema object's `required` set.
///
/// Recursive so a nested object is filled the same way its parent is, which
/// is what lets the test keep checking schema-against-type as the shape
/// grows rather than needing a new hand-written literal each time.
fn required_shape(schema: &Value) -> Value {
    match schema["type"].as_str() {
        Some("array") => json!([required_shape(&schema["items"])]),
        Some("object") => {
            let mut object = serde_json::Map::new();
            for name in schema["required"].as_array().into_iter().flatten() {
                let Some(name) = name.as_str() else { continue };
                object.insert(
                    name.to_string(),
                    required_shape(&schema["properties"][name]),
                );
            }
            Value::Object(object)
        }
        Some("integer") => json!(1),
        // The one string field with a closed set of values.
        _ if schema["enum"].is_array() => schema["enum"][0].clone(),
        _ => json!("x"),
    }
}

#[test]
fn schema_and_type_agree_on_required_field_names() {
    let schema = finding_schema();
    let required = schema["properties"]["findings"]["items"]["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = required.iter().filter_map(Value::as_str).collect();

    // A finding shaped exactly like the schema's required set must deserialize.
    let mut object = serde_json::Map::new();
    for name in &names {
        let value = match *name {
            "line" => json!(1),
            "severity" => json!("high"),
            // Built from the schema's own nested required set rather than
            // hand-written, so this keeps checking the two agree instead of
            // freezing today's field names into the test.
            "fix" => required_shape(&fix_schema()),
            _ => json!("x"),
        };
        object.insert((*name).to_string(), value);
    }
    let parsed = serde_json::from_value::<RawFinding>(Value::Object(object));
    assert!(
        parsed.is_ok(),
        "schema required set does not build a RawFinding: {parsed:?}"
    );
}

#[test]
fn empty_findings_array_parses_as_a_clean_sweep() {
    let parsed: RawFindings = serde_json::from_str(r#"{"findings":[]}"#).unwrap_or(RawFindings {
        findings: vec![RawFinding {
            title: "sentinel".into(),
            severity: Severity::Low,
            file: String::new(),
            line: 1,
            snippet: String::new(),
            explanation: String::new(),
            failure_scenario: String::new(),
            fix: Default::default(),
        }],
    });
    assert!(parsed.findings.is_empty());
}

#[test]
fn a_finding_with_no_fix_plan_still_lands_rather_than_sinking_the_sweep() {
    // Only Claude's output is schema-constrained; the others are asked
    // nicely. A model that answers without `fix` must cost us that one
    // field, not the whole sweep it came in.
    let without = r#"{"findings":[{"title":"t","severity":"high","file":"a.rs",
            "line":2,"snippet":"x","explanation":"e","failure_scenario":"f"}]}"#;
    let parsed: RawFindings = serde_json::from_str(without)
        .unwrap_or_else(|e| panic!("a finding without a fix plan was rejected: {e}"));
    assert_eq!(parsed.findings.len(), 1);
    assert!(parsed.findings[0].fix.approach.is_empty());
    assert!(parsed.findings[0].fix.edits.is_empty());
}

#[test]
fn severity_tells_the_model_what_the_levels_mean() {
    // It was a bare enum with no description, and models assigned it by
    // instinct: measured against independent assessment, 6 of 14 severities
    // were wrong, in both directions. Severity is the only thing the report
    // orders by, so an undefined scale made "worst first" unsupportable.
    let schema = finding_schema();
    let severity = &schema["properties"]["findings"]["items"]["properties"]["severity"];
    let text = severity["description"].as_str().unwrap_or_default();
    assert!(!text.is_empty(), "severity has no description at all");
    for level in ["critical", "high", "medium", "low"] {
        assert!(text.contains(level), "{level} is not defined");
    }
    assert!(
        text.contains("workaround"),
        "nothing distinguishes a blocking defect from an inconvenient one"
    );
}

#[test]
fn a_reply_with_no_findings_key_is_malformed_rather_than_a_clean_sweep() {
    // The defect: `#[serde(default)]` on `findings` turned `{}` into a lane
    // that ran and found nothing. Real defects the model had found were
    // lost with no warning, no rejected-finding trail, and no repair
    // attempt — the repair path only fires on a hard deserialize error,
    // which this case never produced.
    let empty_object = serde_json::from_str::<RawFindings>("{}");
    assert!(
        empty_object.is_err(),
        "a reply with no findings key parsed as a clean sweep"
    );

    let wrong_key = serde_json::from_str::<RawFindings>(r#"{"results": []}"#);
    assert!(
        wrong_key.is_err(),
        "a reply that nested its findings elsewhere parsed as a clean sweep"
    );
}

#[test]
fn an_explicitly_empty_list_is_still_a_clean_sweep() {
    // The honest "I looked and found nothing" must keep working, or the
    // fix above would turn every clean lane into a failure.
    let parsed = serde_json::from_str::<RawFindings>(r#"{"findings": []}"#);
    assert!(parsed.is_ok_and(|p| p.findings.is_empty()));
}
