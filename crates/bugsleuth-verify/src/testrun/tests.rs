//! Tests for running tests and classifying the result, split out at the hard
//! line cap.

use super::*;

#[test]
fn a_compile_error_is_not_mistaken_for_a_demonstrated_bug() {
    let outcome = classify(false, "", "error[E0308]: mismatched types");
    assert_eq!(outcome, Outcome::DidNotBuild);
}

#[test]
fn a_real_test_failure_is_recognised() {
    let outcome = classify(
        false,
        "running 1 test\ntest result: FAILED. 0 passed; 1 failed",
        "",
    );
    assert_eq!(outcome, Outcome::Failed);
}

#[test]
fn a_green_run_is_recognised() {
    let outcome = classify(true, "test result: ok. 5 passed; 0 failed", "");
    assert_eq!(outcome, Outcome::Passed);
}

#[test]
fn a_non_zero_exit_with_no_test_output_counts_as_not_built() {
    assert_eq!(classify(false, "", ""), Outcome::DidNotBuild);
}

#[test]
fn passed_tests_reads_the_names_of_the_tests_that_passed() {
    let stdout = "running 2 tests\n\
         test module::old_a ... ok\n\
         test module::old_b ... FAILED\n\n\
         test result: FAILED. 1 passed; 1 failed; 0 ignored";
    let passed = passed_tests(stdout);
    assert!(passed.contains("module::old_a"), "{passed:?}");
    assert!(!passed.contains("module::old_b"), "{passed:?}");
    // The `test result:` summary line must not be mistaken for a test name.
    assert_eq!(passed.len(), 1, "{passed:?}");
}

#[test]
fn ignored_and_summary_lines_are_not_counted_as_passing_tests() {
    let stdout =
        "test a::b ... ok\ntest a::c ... ignored\ntest result: ok. 1 passed; 0 failed; 1 ignored";
    let passed = passed_tests(stdout);
    assert_eq!(
        passed,
        ["a::b".to_string()].into_iter().collect(),
        "{passed:?}"
    );
}

#[test]
fn a_test_name_may_not_carry_shell_or_flag_characters() {
    assert!(is_safe_test_name("html::tests::spaced_src_trips_banner"));
    assert!(!is_safe_test_name("foo; rm -rf /"));
    assert!(!is_safe_test_name("--nocapture"));
    assert!(!is_safe_test_name("foo bar"));
    assert!(!is_safe_test_name(""));
}

#[test]
fn an_unsafe_test_name_is_refused_rather_than_run() {
    let result = run(
        Path::new("."),
        "cargo test",
        Some("foo; whoami"),
        Duration::from_secs(1),
    );
    assert!(matches!(result, Err(TestError::UnsafeTestName(_))));
}

mod private_log_dir_tests {
    use super::super::*;

    /// The log directory must never be inside the repository under review.
    ///
    /// It used to be `<repo>/target/bugsleuth/`. The reviewed repository is
    /// untrusted by design and a proof attempt runs its build, so a symlink at
    /// that path pointed this run's writes wherever the repository chose —
    /// arbitrary file destruction, using our own permissions.
    ///
    /// This fix shipped with no test at all, which an independent audit found
    /// while checking the claim that every fix had one. It has one now.
    #[test]
    fn the_log_directory_is_outside_any_reviewed_repository() {
        let dir = private_log_dir().expect("a private directory");
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "logs land at {dir:?}, which is not under the temp directory and could \
             therefore be inside the tree being reviewed"
        );
        assert!(dir.exists(), "the directory was not actually created");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exclusive creation, so two runs cannot share one directory.
    ///
    /// `create_dir` rather than `create_dir_all` is the whole mechanism: the
    /// failure when the path exists is what proves the directory is ours and
    /// not something a reviewed repository — or a second BugSleuth — put there.
    #[test]
    fn two_runs_never_get_the_same_directory() {
        let first = private_log_dir().expect("first");
        let second = private_log_dir().expect("second");
        assert_ne!(first, second, "two runs would write over each other");
        for dir in [&first, &second] {
            assert!(dir.exists());
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// And an existing directory is stepped over rather than adopted.
    #[test]
    fn a_directory_already_there_is_not_taken_over() {
        // Occupy what the next call would otherwise choose, then confirm the
        // call still returns something, and something else.
        let taken = private_log_dir().expect("seed");
        let next = private_log_dir().expect("after");
        assert_ne!(taken, next);
        assert!(next.exists());
        for dir in [&taken, &next] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
