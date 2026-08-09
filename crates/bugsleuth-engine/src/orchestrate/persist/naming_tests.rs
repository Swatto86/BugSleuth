//! Tests for the report-filename encoding, split from the resume tests only
//! because the two together crossed the hard line cap. These need nothing but
//! `file_name_for`/`legacy_file_name_for`; the resume tests need a git repo.

use super::super::persist::*;
use bugsleuth_domain::Lane;

fn unit() -> Unit {
    Unit {
        model: "claude:sonnet".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    }
}

#[test]
fn each_unit_gets_a_distinct_file_so_sweeps_cannot_overwrite_each_other() {
    let a = file_name_for(&unit());
    let b = file_name_for(&Unit {
        model: "codex:".into(),
        lane: Lane::Correctness,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    });
    let c = file_name_for(&Unit {
        model: "claude:sonnet".into(),
        lane: Lane::Security,
        effort: String::new(),
        use_agents: false,
        pass: 1,
    });
    assert_ne!(a, b);
    assert_ne!(a, c);
}

#[test]
fn an_agent_sweep_cannot_reuse_or_overwrite_a_single_agent_sweep() {
    let single = file_name_for(&unit());
    let agents = file_name_for(&Unit {
        use_agents: true,
        ..unit()
    });
    assert_ne!(single, agents);
    assert!(agents.contains("~agents"), "got {agents}");
}

#[test]
fn a_second_pass_writes_beside_the_first_rather_than_over_it() {
    // The whole value of repetition is keeping both results: three
    // identical sweeps of one fixture found five findings each but six
    // between them. Overwriting would buy nothing.
    let first = file_name_for(&unit());
    let second = file_name_for(&Unit { pass: 2, ..unit() });
    assert_ne!(first, second);
    assert!(second.contains("~p2"), "got {second}");
    // A first pass keeps the historical name, so reports written before
    // passes existed still resume.
    assert!(!first.contains("pass"), "got {first}");
}

#[test]
fn an_effort_spelling_the_pass_suffix_cannot_collide_with_a_real_second_pass() {
    // Both of these are reachable from ordinary config values: effort is
    // free text at every entry point. Under plain concatenation they were
    // byte-identical, so whichever sweep finished last silently overwrote
    // the other and resume handed one unit the other's report.
    let second_pass = file_name_for(&Unit { pass: 2, ..unit() });
    let odd_effort = file_name_for(&Unit {
        effort: "pass2".into(),
        ..unit()
    });
    assert_ne!(second_pass, odd_effort);
    // The same trap from the other side: an effort of "p2" against the
    // new-style pass marker.
    let p2_effort = file_name_for(&Unit {
        effort: "p2".into(),
        ..unit()
    });
    assert_ne!(second_pass, p2_effort);
}

#[test]
fn two_model_ids_that_differ_only_in_punctuation_get_different_files() {
    // `codex:a/b` and `codex:a-b` both used to render as `codex-a-b`: one
    // sweep overwrote the other, and a resumed run handed a model the other
    // model's findings while the report claimed the wrong provenance.
    let slash = file_name_for(&Unit {
        model: "codex:a/b".into(),
        ..unit()
    });
    let dash = file_name_for(&Unit {
        model: "codex:a-b".into(),
        ..unit()
    });
    assert_ne!(slash, dash);
}

#[test]
fn variable_length_hex_escapes_cannot_collide_with_literal_hex_digits() {
    let split = file_name_for(&Unit {
        model: "_2d".into(),
        ..unit()
    });
    let joined = file_name_for(&Unit {
        model: "\u{5F2D}".into(),
        ..unit()
    });
    assert_ne!(split, joined);
}

#[test]
fn an_encoded_name_can_never_reach_outside_the_run_directory() {
    // The encoding exists to be injective, but it must not have bought that
    // by letting a separator or a parent reference through.
    for hostile in ["../../etc/passwd", r"C:\Windows\System32", r"a/b\c", ".."] {
        let name = file_name_for(&Unit {
            model: hostile.to_string(),
            ..unit()
        });
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert!(!name.contains(".."), "{name}");
        assert!(!name.contains(':'), "{name}");
    }
}
