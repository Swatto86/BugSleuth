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
    let scratch = dir.join("target").join("bugsleuth");
    let _ = std::fs::create_dir_all(&scratch);
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

    let outcome = match status {
        Some(status) => classify(status.success(), &stdout, &stderr),
        // A test suite that never finished proves nothing about the code.
        None => Outcome::TimedOut,
    };

    Ok(TestRun {
        outcome,
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

/// Cargo test filters are module paths: alphanumerics, `_`, and `::`.
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
