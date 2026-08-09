//! Tests for the severity triage pass, in their own file only because the
//! module plus its tests crossed the hard line cap.

use super::*;

use bugsleuth_domain::{Finding, FindingId, Lane, ModelId, RawFinding, VerifiedAnchor};

fn cluster_at(severity: Severity, title: &str) -> Cluster {
    Cluster {
        findings: vec![Finding::new(
            FindingId::new("f"),
            Lane::Correctness,
            ModelId::new("claude:sonnet"),
            RawFinding {
                title: title.into(),
                severity,
                file: "a.rs".into(),
                line: 1,
                snippet: "x".into(),
                explanation: "e".into(),
                failure_scenario: "boom".into(),
                fix: Default::default(),
            },
            VerifiedAnchor {
                file: "a.rs".into(),
                line: 1,
                claimed_line: 1,
                snippet: "x".into(),
            },
        )],
        agreement: 1,
        acknowledged: None,
        triaged: None,
        triage_reason: None,
    }
}

#[test]
fn every_defect_is_offered_under_an_id_the_reply_can_name() {
    let clusters = vec![
        cluster_at(Severity::Low, "first"),
        cluster_at(Severity::High, "second"),
    ];
    let prompt = prompt_for(&clusters, None);
    assert!(prompt.contains("## id 1"));
    assert!(prompt.contains("## id 2"));
    assert!(prompt.contains("first"));
    assert!(prompt.contains("second"));
}

#[test]
fn the_prompt_says_what_each_defect_was_already_graded() {
    // Withholding it would not buy independence, only a judgement made with
    // less to go on.
    let prompt = prompt_for(&[cluster_at(Severity::Critical, "t")], None);
    assert!(prompt.contains("Currently graded: critical"), "{prompt}");
}

#[test]
fn the_prompt_forbids_the_three_things_that_would_damage_the_report() {
    let prompt = prompt_for(&[cluster_at(Severity::Low, "t")], None);
    for rule in ["Do not add defects", "Do not remove", "Do not merge"] {
        assert!(prompt.contains(rule), "the prompt does not say: {rule}");
    }
}

#[tokio::test]
async fn a_single_defect_is_not_paid_to_be_compared_with_nothing() {
    // Comparison is the whole mechanism; with one defect there is nothing to
    // compare and the call could only restate the model's own opinion.
    let mut clusters = vec![cluster_at(Severity::Low, "only")];
    let outcome = apply(
        &mut clusters,
        Request {
            repo: Path::new("."),
            // A binary that does not exist: if this ever tried to run, the
            // test would fail rather than quietly spending quota.
            model: "no-such-model",
            effort: "",
            timeout: Duration::from_secs(1),
            api_key: None,
        },
    )
    .await;
    assert_eq!(outcome, Outcome::default());
    assert_eq!(clusters[0].triaged, None);
}

#[tokio::test]
async fn cancelled_run_does_not_start_or_wait_for_triage() {
    // Stopping a run must stop the triage pass too. With the token already
    // stopped, grade returns immediately as ungraded — the biased race means the
    // provider future is never polled, so no model process is started — and no
    // cluster is graded.
    let cancel = crate::cancel::Cancel::new();
    cancel.stop();
    let options = crate::orchestrate::RunOptions {
        repo: Path::new("."),
        scope: None,
        max_turns: 1,
        timeout: Duration::from_secs(1),
        api_key: None,
        out_dir: None,
        resume: false,
        progress: None,
        cancel,
        // Nonempty, so the "no triage model" guard does not short-circuit and
        // hide the cancellation path being exercised.
        triage_model: "claude:sonnet",
        per_vendor_concurrency: 1,
    };
    let mut clusters = vec![
        cluster_at(Severity::Low, "first"),
        cluster_at(Severity::High, "second"),
    ];

    let outcome = grade(&mut clusters, &options).await;
    assert!(
        outcome.note.contains("cancelled"),
        "a cancelled run did not report itself cancelled: {}",
        outcome.note
    );
    assert_eq!(outcome.graded, 0, "a cancelled run graded a defect");
    assert!(
        clusters.iter().all(|cluster| cluster.triaged.is_none()),
        "a cancelled run still triaged a cluster"
    );
}

#[test]
fn the_prompt_never_offers_tools_the_pass_does_not_have() {
    // It once did, for a whole afternoon of runs. Told to read files it had
    // no way to open, the pass burned every turn and returned nothing at all
    // — and the failure looked exactly like a turn limit set too low.
    let prompt = prompt_for(&[], None);
    for offer in ["You may read", "open the file", "Read only what you need"] {
        assert!(!prompt.contains(offer), "the prompt still offers: {offer}");
    }
    assert!(prompt.contains("no tools"), "{prompt}");
}

#[test]
fn a_recovered_grading_is_not_presented_as_a_full_comparison() {
    // The whole mechanism is comparison. A grading model that was cut off
    // and asked afterwards what it had decided compared some of the list,
    // and the report has to say which. The wording is built by grading_note,
    // so this assertion guards the real production code, not a hand-written copy.
    let note = grading_note(true, 4, 9);
    assert!(note.contains("recovered afterwards"), "{note}");
    assert!(note.contains("interrupted"), "{note}");
    assert!(note.contains("rather than all of them"), "{note}");
    assert!(note.contains("4 of 9"), "{note}");
}

fn scratch(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("bugsleuth-ack-tests")
        .join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("scratch");
    std::fs::write(dir.join("src/x.rs"), body).expect("write");
    dir
}

fn cluster() -> Cluster {
    Cluster {
        findings: vec![Finding::new(
            FindingId::new("f"),
            Lane::Security,
            ModelId::new("claude:sonnet"),
            RawFinding {
                title: "t".into(),
                severity: Severity::High,
                file: "src/x.rs".into(),
                line: 3,
                snippet: "code".into(),
                explanation: "e".into(),
                failure_scenario: "f".into(),
                fix: Default::default(),
            },
            VerifiedAnchor {
                file: "src/x.rs".into(),
                line: 3,
                claimed_line: 3,
                snippet: "code".into(),
            },
        )],
        agreement: 1,
        acknowledged: None,
        triaged: None,
        triage_reason: None,
    }
}

const SOURCE: &str = "// Kilo cannot be restricted per invocation, so it is\n\
                          // pointed at an empty private directory instead.\n\
                          fn code() {}\n";

/// A dismissal backed by words really in the file is accepted.
#[test]
fn a_quote_that_is_in_the_file_settles_it() {
    let dir = scratch("real", SOURCE);
    let reason = "ACKNOWLEDGED: \"pointed at an empty private directory instead\" — \
                      a deliberate trade-off.";
    let got = acknowledgement(reason, &dir, &cluster());
    assert!(
        got.is_some(),
        "a quote that is genuinely there was rejected"
    );
    assert!(got.unwrap().0.contains("empty private directory"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// And one that is not is discarded, with the finding left standing.
///
/// The property this whole mechanism rests on. Without it, the cheapest way
/// to make a report look clean would be to claim every defect was already
/// known — so a dismissal carries its own proof exactly as a finding does.
#[test]
fn a_quote_that_is_not_in_the_file_is_refused() {
    let dir = scratch("invented", SOURCE);
    let reason = "ACKNOWLEDGED: \"this risk was reviewed and formally accepted\" — trust me.";
    assert!(
        acknowledgement(reason, &dir, &cluster()).is_none(),
        "an invented quote was accepted as proof the code already knew"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_verdict_making_no_such_claim_is_left_alone() {
    let dir = scratch("plain", SOURCE);
    assert!(acknowledgement("A real defect, high.", &dir, &cluster()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_quote_too_short_to_be_evidence_is_refused() {
    let dir = scratch("short", SOURCE);
    assert!(acknowledgement("ACKNOWLEDGED: \"so it is\"", &dir, &cluster()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The comment above a finding is what gets offered to the grading model.
#[test]
fn the_prompt_carries_what_the_code_says_about_itself() {
    let dir = scratch("prompt", SOURCE);
    let clusters = vec![cluster(), cluster()];
    let prompt = prompt_for(&clusters, Some(&dir));
    assert!(
        prompt.contains("What the code says about itself here"),
        "{prompt}"
    );
    assert!(prompt.contains("empty private directory"), "{prompt}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rust_attributes_are_skipped_instead_of_quoted_as_commentary() {
    let dir = scratch(
        "attributes",
        "// the real reason\n#[serde(default)]\nfn code() {}\n",
    );
    let finding = cluster();
    let commentary = commentary_at(&dir, &finding.findings[0]).expect("comment above item");
    assert!(commentary.contains("the real reason"), "{commentary}");
    assert!(!commentary.contains("#[serde"), "{commentary}");
    let _ = std::fs::remove_dir_all(&dir);

    let dir = scratch(
        "attributes-only",
        "#[derive(Debug)]\n#[serde(default)]\nfn code() {}\n",
    );
    let finding = cluster();
    assert!(commentary_at(&dir, &finding.findings[0]).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
