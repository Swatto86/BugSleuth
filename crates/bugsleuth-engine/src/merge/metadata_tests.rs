use super::super::merge::*;
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bugsleuth-merge-metadata-{name}-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn reports_of_different_scopes_are_not_merged_as_one_review() {
    let dir = scratch("mixed-scope");
    let first = write(
        &dir,
        "first.json",
        r#"{
          "lane":"Correctness", "model":"claude:sonnet", "scope":"src/engine",
          "status":{"state":"swept"}, "findings":[], "rejected":[]
        }"#,
    );
    let second = write(
        &dir,
        "second.json",
        r#"{
          "lane":"Security", "model":"codex:gpt-5", "scope":"src/ui",
          "status":{"state":"swept"}, "findings":[], "rejected":[]
        }"#,
    );

    let error = merge(&[first, second])
        .err()
        .expect("mixed scopes must fail");

    assert!(error.to_string().contains("different scopes"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rejected_findings_remain_visible_after_merge() {
    let dir = scratch("source-metadata");
    let pinned = write(
        &dir,
        "pinned.json",
        r#"{
          "lane":"Correctness", "model":"claude:sonnet",
          "commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "cache_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "scope":"src/engine", "usage":"input_tokens=12 output_tokens=3",
          "excluded_paths":[".cursor"],
          "status":{"state":"swept"}, "findings":[],
          "rejected":[{"future_report_shape":{"reason":"not part of SweepFile schema"}}]
        }"#,
    );
    let unpinned = write(
        &dir,
        "unpinned.json",
        r#"{
          "lane":"Security", "model":"codex:gpt-5",
          "commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
          "scope":"src/engine", "usage":"input_tokens=8 output_tokens=2",
          "status":{"state":"swept"}, "findings":[], "rejected":[]
        }"#,
    );

    let merged = merge(&[pinned, unpinned]).unwrap();
    let rendered = merged.to_text();

    assert!(rendered.contains("scope: src/engine"));
    assert!(rendered.contains("0 verified, 1 rejected"));
    assert!(rendered.contains("input_tokens=12 output_tokens=3"));
    assert!(rendered.contains("revision aaaaaaa, pinned"));
    assert!(rendered.contains("revision bbbbbbb, unpinned"));
    assert!(rendered.contains("Not reviewed because provider isolation removed: .cursor"));
    assert!(rendered.contains("rejected claims"));
    let prompt = merged.to_fix_prompt("repo");
    assert!(prompt.contains(".cursor"));
    assert!(prompt.contains("provider isolation removed it"));
    assert!(!prompt.contains("future_report_shape"));
    let _ = fs::remove_dir_all(dir);
}
