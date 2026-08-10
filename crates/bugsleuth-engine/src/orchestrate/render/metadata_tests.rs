use crate::orchestrate::{RunReport, Swept};
use bugsleuth_domain::Lane;

fn sweep(commit: &str, cached: Option<&str>, scope: Option<&str>, usage: Option<&str>) -> Swept {
    Swept {
        model: "claude:sonnet".into(),
        lane: Lane::Correctness,
        commit: Some(commit.into()),
        cache_revision: cached.map(str::to_string),
        scope: scope.map(str::to_string),
        usage: usage.map(str::to_string),
        findings: 0,
        rejected: 0,
        salvaged: false,
    }
}

fn report(swept: Vec<Swept>) -> RunReport {
    RunReport {
        ranked: vec![],
        triage: Default::default(),
        swept,
        gaps: vec![],
    }
}

#[test]
fn run_report_calls_out_mixed_revisions() {
    let text = report(vec![
        sweep("aaaaaaaa11111111", Some("aaaaaaaa11111111"), None, None),
        sweep("bbbbbbbb22222222", Some("bbbbbbbb22222222"), None, None),
    ])
    .to_text();
    assert!(text.contains("WARNING"), "{text}");
    assert!(text.contains("2 different revisions"), "{text}");
    assert!(text.contains("aaaaaaa"), "{text}");
    assert!(text.contains("bbbbbbb"), "{text}");
}

#[test]
fn run_report_preserves_usage() {
    let text = report(vec![sweep(
        "aaaaaaaa11111111",
        None,
        Some("src/engine"),
        Some("input_tokens=900 output_tokens=90"),
    )])
    .to_text();
    assert!(text.contains("scope: src/engine"), "{text}");
    assert!(text.contains("revision aaaaaaa, unpinned"), "{text}");
    assert!(
        text.contains("usage: input_tokens=900 output_tokens=90"),
        "{text}"
    );
    assert!(
        text.contains("consistency across this run is unconfirmed"),
        "{text}"
    );
}

#[test]
fn mixed_scopes_are_not_rendered_as_one_ordinary_run() {
    let report = report(vec![
        sweep(
            "aaaaaaaa11111111",
            Some("aaaaaaaa11111111"),
            Some("src/a"),
            None,
        ),
        sweep(
            "aaaaaaaa11111111",
            Some("aaaaaaaa11111111"),
            Some("src/b"),
            None,
        ),
    ]);
    assert!(super::super::super::common_scope(&report.swept).is_err());
    let text = report.to_text();
    assert!(text.contains("WARNING"), "{text}");
    assert!(text.contains("different review scopes"), "{text}");
    assert!(!text.contains("scope: src/a"), "{text}");
}
