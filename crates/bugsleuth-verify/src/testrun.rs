//! Running tests, and deciding what the result proves.
//!
//! The whole point of a proof-carrying finding is that *we* run the test, not
//! the model. A model's report that its test failed is just another claim; the
//! exit status of a process is evidence.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("could not run the test command `{command}`: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the test name `{0}` contains characters that are not allowed in a test filter")]
    UnsafeTestName(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every test the command ran passed.
    Passed,
    /// At least one test failed. For a proof attempt, this is the good result.
    Failed,
    /// The command did not get as far as running tests — usually a compile error.
    DidNotBuild,
    /// The suite was killed for running too long. Proves nothing either way.
    TimedOut,
}

#[derive(Debug, Clone)]
pub struct TestRun {
    pub outcome: Outcome,
    pub stdout: String,
    pub stderr: String,
    /// The identities of the tests that passed, e.g. `module::name`. This is
    /// how loss of a previously passing test is detected: a count cannot, because
    /// a proof attempt that breaks one existing test but adds a new passing one
    /// leaves the total unchanged. Only the set of names shows the loss.
    pub passed_tests: std::collections::BTreeSet<String>,
}

impl TestRun {
    /// A short quotable summary for the report.
    pub fn summary(&self) -> String {
        let line = self
            .stdout
            .lines()
            .find(|line| line.starts_with("test result:"))
            .or_else(|| {
                self.stderr
                    .lines()
                    .find(|line| line.starts_with("error") || line.contains("error["))
            })
            .unwrap_or("no test summary line");
        line.chars().take(300).collect()
    }
}

/// Run a test command in `dir`, optionally filtered to a single test.
///
/// `base_command` is split on whitespace and run directly — no shell. The
/// filter is checked against a conservative character set first, because it is
/// the one part of the command line that came from a model, and a model that
/// returns `foo; rm -rf /` must not be able to turn a test filter into a shell
/// instruction. Running without a shell already prevents that; validating as
/// well means a hostile name is rejected loudly rather than silently mangled.
pub fn run(
    dir: &Path,
    base_command: &str,
    filter: Option<&str>,
    timeout: Duration,
) -> Result<TestRun, TestError> {
    let mut parts = base_command.split_whitespace();
    let program = parts.next().unwrap_or("cargo");
    let mut args: Vec<String> = parts.map(str::to_string).collect();

    if let Some(filter) = filter {
        if !is_safe_test_name(filter) {
            return Err(TestError::UnsafeTestName(filter.to_string()));
        }
        args.push(filter.to_string());
    }

    // Output goes to files, not pipes. A model-written test can produce an
    // unbounded amount of output, and a pipe that fills up while we are not
    // reading it would block the child forever — the exact hang the timeout
    // below exists to prevent.
    // **Outside the reviewed tree.** These used to be written to
    // `<repo>/target/bugsleuth/`, which the repository under review controls:
    // a committed symlink at that path is materialised on checkout, and
    // `File::create` follows it and truncates whatever it points at. That is
    // arbitrary file destruction chosen by the code being reviewed — the same
    // escape the anchor check was hardened against, in the one place that
    // writes rather than reads.
    //
    // Created exclusively so the directory is ours because creating it proved
    // it did not exist, and removed when the run is done.
    let scratch = private_log_dir()?;
    let out_path = scratch.join("test-stdout.log");
    let err_path = scratch.join("test-stderr.log");
    let open = |path: &Path| {
        std::fs::File::create(path).map_err(|source| TestError::Spawn {
            command: base_command.to_string(),
            source,
        })
    };

    let mut child = crate::console::hide(&mut Command::new(program))
        .args(&args)
        .current_dir(dir)
        // Colour codes would corrupt the parsing below and clutter the report.
        .env("CARGO_TERM_COLOR", "never")
        .env("RUST_BACKTRACE", "0")
        .stdout(open(&out_path)?)
        .stderr(open(&err_path)?)
        .spawn()
        .map_err(|source| TestError::Spawn {
            command: base_command.to_string(),
            source,
        })?;

    let status = wait_with_timeout(&mut child, timeout);
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    // Ours, and nobody else's business once read.
    let _ = std::fs::remove_dir_all(&scratch);

    let outcome = match status {
        Some(status) => classify(status.success(), &stdout, &stderr),
        // A test suite that never finished proves nothing about the code.
        None => Outcome::TimedOut,
    };

    Ok(TestRun {
        outcome,
        passed_tests: passed_tests(&stdout),
        stdout,
        stderr,
    })
}

/// Wait for the child, killing it if it outstays `timeout`. `None` means it was
/// killed. Polling rather than blocking, so a model that writes an infinite loop
/// into a test cannot hang the whole run.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Total passed and failed across every test binary the command ran.
///
/// Cargo prints one `test result:` line per binary, so a workspace run produces
/// several and they have to be summed. These counts are how sabotage is
/// detected: if fewer tests pass after a proof attempt than before it, the model
/// changed production code instead of only adding a test, and its new failing
/// test is no longer evidence about the original defect.
pub fn counts(stdout: &str) -> (u32, u32) {
    let mut passed = 0;
    let mut failed = 0;
    for line in stdout.lines().filter(|l| l.starts_with("test result:")) {
        let mut words = line.split_whitespace().peekable();
        while let Some(word) = words.next() {
            let Ok(count) = word.parse::<u32>() else {
                continue;
            };
            match words.peek() {
                Some(&"passed;") | Some(&"passed") => passed += count,
                Some(&"failed;") | Some(&"failed") => failed += count,
                _ => {}
            }
        }
    }
    (passed, failed)
}

/// The identities of the tests that passed, parsed from libtest output lines
/// of the form `test module::name ... ok`.
///
/// The counterpart to [`counts`], and the one that can detect a lost test. A
/// proof attempt that breaks one existing test but adds a new passing one keeps
/// the passing *count* the same, so only comparing the set of names before and
/// after shows that a previously passing test no longer passes.
pub fn passed_tests(stdout: &str) -> std::collections::BTreeSet<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("test ")?;
            rest.strip_suffix(" ... ok").map(str::to_string)
        })
        .collect()
}

/// Distinguish "the tests ran and something failed" from "nothing ran".
///
/// Both exit non-zero, and conflating them would be a serious error: a proof
/// attempt that merely broke the build would otherwise be scored as a
/// successfully demonstrated bug.
fn classify(success: bool, stdout: &str, stderr: &str) -> Outcome {
    if success {
        return Outcome::Passed;
    }
    let ran_tests = stdout.contains("test result:") || stdout.contains("running ");
    let compile_error = stderr.contains("error[E")
        || stderr.contains("error: could not compile")
        || stderr.contains("could not compile");

    if compile_error && !ran_tests {
        Outcome::DidNotBuild
    } else if ran_tests {
        Outcome::Failed
    } else {
        Outcome::DidNotBuild
    }
}

/// A private directory for one test run's output, outside the reviewed tree.
///
/// `create_dir` rather than `create_dir_all`: the failure when the path already
/// exists is the point — it is what proves the directory is ours rather than
/// something a reviewed repository, or another process, put there first.
fn private_log_dir() -> Result<std::path::PathBuf, TestError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);

    let base = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);

    for attempt in 0..64 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("bugsleuth-testrun-{pid}-{nanos:08x}-{n}-{attempt}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(TestError::Spawn {
                    command: "creating a private log directory".to_string(),
                    source,
                });
            }
        }
    }
    Err(TestError::Spawn {
        command: "creating a private log directory".to_string(),
        source: std::io::Error::other("no unused name was available"),
    })
}

fn is_safe_test_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

#[cfg(test)]
mod tests {
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
        let stdout = "test a::b ... ok\ntest a::c ... ignored\ntest result: ok. 1 passed; 0 failed; 1 ignored";
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
}

#[cfg(test)]
mod private_log_dir_tests {
    use super::*;

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
