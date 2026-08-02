use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bugsleuth-atomic-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

#[test]
fn a_file_is_replaced_whole() {
    let dir = scratch("replace");
    let path = dir.join("report.json");
    write(&path, "first").expect("first write");
    write(&path, "second").expect("second write");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "second");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_left_beside_the_file_afterwards() {
    // A `.writing` file left behind is read by the next directory scan as a
    // report whose name matches no unit.
    let dir = scratch("leftovers");
    write(&dir.join("report.json"), "body").expect("write");
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["report.json".to_string()], "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_previous_file_survives_a_failed_write() {
    // The whole point. A directory in place of the staging file makes the write
    // fail exactly where the old code had already truncated the target.
    let dir = scratch("failure");
    let path = dir.join("report.json");
    std::fs::write(&path, "the good one").expect("seed");

    let staged = staged_path(&path).expect("staged path");
    std::fs::create_dir_all(&staged).expect("block the staging path");

    let result = write(&path, "the replacement");
    assert!(result.is_err(), "the write should have failed");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "the good one",
        "the previous file was destroyed by a write that failed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_staging_file_is_a_sibling_so_the_rename_cannot_cross_a_filesystem() {
    let path = Path::new("/tmp/reports/run.json");
    let staged = staged_path(path).expect("staged path");
    assert_eq!(staged.parent(), path.parent());
    assert!(is_staged(&staged.file_name().unwrap().to_string_lossy()));
}

#[test]
fn a_path_that_names_no_file_is_an_error_rather_than_a_panic() {
    assert!(staged_path(Path::new("/")).is_err());
}

/// The reason this module exists: the same six lines written out three times,
/// and a fourth place that never got them. Anything writing a file a person
/// would mind losing has to come through here.
#[test]
fn no_durable_write_bypasses_this_module() {
    let documents = [
        ("the run reports", include_str!("../orchestrate/persist.rs")),
        ("the fix prompt", include_str!("../handoff.rs")),
    ];
    for (name, source) in documents {
        // Comments go first. These modules explain at length why `fs::write`
        // is the wrong call, and a scan that reads prose as code fails on the
        // explanation for the rule it is enforcing.
        let code: String = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(before, _)| before)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("fs::write"),
            "{name} writes a durable file directly instead of through atomic::write"
        );
        assert!(
            code.contains("atomic::write"),
            "{name} no longer writes anything through atomic::write, which cannot be right"
        );
    }
}
