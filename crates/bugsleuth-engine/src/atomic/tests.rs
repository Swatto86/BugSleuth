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
fn a_failed_write_destroys_nothing_and_leaves_no_debris() {
    // The whole point of the module. The failure is forced at the rename —
    // a directory sits where the file should go, so `fs::rename` cannot
    // replace it — because the staging path is now unique per writer and a
    // test can no longer predict it to block it. That uniqueness is itself a
    // fix: two writers used to share one staging file.
    let dir = scratch("failure");
    let target = dir.join("report.json");
    std::fs::create_dir_all(&target).expect("block the target");
    std::fs::write(target.join("kept.txt"), "the good one").expect("seed");

    let result = write(&target, "the replacement");
    assert!(result.is_err(), "the write should have failed");
    assert_eq!(
        std::fs::read_to_string(target.join("kept.txt")).expect("read"),
        "the good one",
        "a write that failed destroyed what was already there"
    );

    let debris: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| is_staged(name))
        .collect();
    assert!(debris.is_empty(), "a failed write left {debris:?} behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_stage_write_leaves_no_debris() {
    // The other failure test forces the rename to fail; this forces the staging
    // write itself to fail after it has created and partially filled the file —
    // a full disk. The `.writing` file must be cleaned, and the existing target
    // left untouched.
    let dir = scratch("stage-failure");
    let target = dir.join("report.json");
    std::fs::write(&target, "PREVIOUS GOOD").expect("seed the target");

    let result = write_with(&target, |staged| {
        std::fs::write(staged, b"partial").expect("partial stage write");
        Err(io::Error::other("disk full"))
    });
    assert!(result.is_err(), "the staging write should have failed");
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        "PREVIOUS GOOD",
        "a failed staging write destroyed the existing file"
    );

    let debris: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| is_staged(name))
        .collect();
    assert!(
        debris.is_empty(),
        "a failed staging write left {debris:?} behind"
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

/// Two writers replacing the same file must not share a staging file.
///
/// The defect, in this module's own first version: the staging name was
/// `.{name}.writing` and nothing else, so two processes replacing one report
/// wrote to the same temporary file at the same time. Each rename then
/// succeeded and the surviving file could hold the other writer's bytes — an
/// atomic write that publishes a mixture, which is worse than none, because the
/// whole promise is that a reader never sees one.
#[test]
fn two_writers_of_one_file_stage_into_different_places() {
    let target = Path::new("/tmp/reports/run.json");
    let first = staged_path(target).expect("first");
    let second = staged_path(target).expect("second");
    assert_ne!(first, second, "both writers chose the same staging file");
    assert_eq!(
        first.parent(),
        target.parent(),
        "staging left the directory"
    );
    assert_eq!(
        second.parent(),
        target.parent(),
        "staging left the directory"
    );
    for path in [&first, &second] {
        assert!(is_staged(&path.file_name().unwrap().to_string_lossy()));
    }
}

/// And concurrently, for real, against one path.
#[test]
fn concurrent_writes_leave_one_whole_file_and_no_debris() {
    let dir = scratch("concurrent");
    let path = dir.join("report.json");
    let bodies = ["a".repeat(20_000), "b".repeat(20_000)];

    std::thread::scope(|scope| {
        for body in &bodies {
            scope.spawn(|| {
                let _ = write(&path, body.as_bytes());
            });
        }
    });

    let landed = std::fs::read_to_string(&path).expect("one of them must have landed");
    assert!(
        bodies.contains(&landed),
        "the file holds a mixture of both writers rather than either one whole"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| is_staged(name))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging files left behind: {leftovers:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
