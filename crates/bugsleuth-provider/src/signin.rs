//! Proving a vendor CLI is actually signed in.
//!
//! **A CLI that starts is not a CLI you can use.** `--version` answers whether
//! the binary is installed; it says nothing about whether the machine holds a
//! session, and these tools exit zero on `--version` while signed out. So the
//! Providers panel could show three green ticks and every sweep then fail on
//! authentication — tens of minutes into a run already committed to, with the
//! quota spent.
//!
//! Authentication is only observable by making a real call, so this makes one:
//! the smallest possible round trip, asking for a single word back. The design
//! is borrowed from Eir, which drives the same CLIs under the same user
//! subscriptions and settles the same question the same way — one trivial
//! prompt, a timeout, and a report of exactly what came back.
//!
//! Deliberately **on demand, not at launch**. This costs real quota, in a tool
//! whose whole design treats a model invocation as expensive; checking at every
//! start would spend three calls to tell most people what they already know.

use std::path::Path;
use std::time::Duration;

use crate::ProviderError;
use crate::process::{CliOutput, Invocation, ProcessError};

/// What a sign-in check found, in the words the interface will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignIn {
    /// A real call went out and came back. The only positive worth reporting.
    Working,
    /// The CLI answered, but with nothing usable. Not proof of a bad session
    /// and not proof of a good one, so it is its own state rather than rounded
    /// to either — rounding it would be a guess presented as a finding, which
    /// is the thing this whole project exists to avoid.
    Empty,
    /// The call failed. Carries the vendor's own message, which is the part
    /// that actually says whether to log in, upgrade, or wait out a limit.
    Failed(String),
    /// No answer in the time allowed.
    TimedOut(u64),
}

impl SignIn {
    /// Whether a sweep would get an answer from this vendor right now.
    #[must_use]
    pub fn usable(&self) -> bool {
        matches!(self, SignIn::Working)
    }

    /// One line for a person, naming the vendor.
    #[must_use]
    pub fn describe(&self, vendor: &str) -> String {
        match self {
            SignIn::Working => format!("{vendor}: signed in and answering"),
            SignIn::Empty => {
                format!("{vendor}: answered with nothing — cannot tell whether it is signed in")
            }
            SignIn::Failed(why) => format!("{vendor}: not usable — {why}"),
            SignIn::TimedOut(seconds) => {
                format!("{vendor}: no answer within {seconds}s, so a sweep would hang too")
            }
        }
    }
}

/// The smallest prompt that proves a session works.
///
/// One word, no tools, no schema. Anything longer is quota spent to learn what
/// this already answers.
pub(crate) const PROMPT: &str = "Reply with exactly OK and nothing else.";

/// How long to wait. Short on purpose: someone is watching this, and a vendor
/// that cannot answer a one-word prompt inside a minute is not one to start a
/// forty-minute sweep against.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(60);

fn answer_from(
    out: CliOutput,
    vendor: &'static str,
    answer: fn(&str) -> String,
) -> Result<String, ProviderError> {
    // A failed exit is the usual shape of "not signed in", and the CLI's own
    // words are the part worth keeping: they say what to do about it.
    if !out.succeeded() {
        let said = out.stderr.trim();
        let message = if said.is_empty() {
            out.stdout.trim().to_string()
        } else {
            said.to_string()
        };
        return Err(ProviderError::Failed {
            vendor,
            code: out.code.unwrap_or(-1),
            message,
        });
    }
    Ok(answer(&out.stdout))
}

/// Ask every vendor for one word, concurrently.
///
/// Concurrent because these are independent processes and a person is waiting:
/// three sequential timeouts is three minutes of spinner to learn what one
/// minute can tell you.
pub async fn check_all() -> Vec<(&'static str, SignIn)> {
    let (claude, codex, kilo) = tokio::join!(
        crate::claude::signin(),
        crate::codex::signin(),
        crate::kilo::signin()
    );
    vec![("claude", claude), ("codex", codex), ("kilo", kilo)]
}

/// Run one CLI's smallest invocation and read what happened.
///
/// Shared by the three adapters so they cannot drift in how an answer is
/// interpreted. The only difference between them is which flags mean "one shot,
/// print it, and stop".
pub(crate) async fn one_shot(
    binary: &str,
    args: &[String],
    vendor: &'static str,
    // How to get the model's actual words out of whatever the CLI printed.
    // Kilo answers in NDJSON, so "stdout was not empty" is satisfied by its own
    // event logging even when no answer was produced — which would report a
    // session as working on the strength of its own noise, exactly the rounding
    // `Empty` exists to prevent. The others print the answer directly.
    answer: fn(&str) -> String,
) -> SignIn {
    let outcome = crate::process::run(Invocation {
        binary,
        args,
        cwd: Path::new("."),
        stdin: Some(PROMPT.as_bytes()),
        env: &[],
        timeout: TIMEOUT,
        what: vendor,
    })
    .await;

    let text = outcome
        .map_err(ProviderError::from)
        .and_then(|out| answer_from(out, vendor, answer));

    classify(text, TIMEOUT.as_secs())
}

/// Turn a raw invocation result into the state to report.
///
/// Separate from [`one_shot`] so the reading of what happened can be tested
/// without spending anything.
pub(crate) fn classify(outcome: Result<String, ProviderError>, seconds: u64) -> SignIn {
    match outcome {
        Ok(text) if text.trim().is_empty() => SignIn::Empty,
        Ok(_) => SignIn::Working,
        Err(ProviderError::Process(ProcessError::Timeout { .. })) => SignIn::TimedOut(seconds),
        Err(error) => SignIn::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_exit_code_is_a_failed_probe() {
        let output = crate::process::CliOutput {
            code: None,
            stdout: "OK".into(),
            stderr: "killed".into(),
        };
        let result = answer_from(output, "test", str::to_string);
        assert!(matches!(
            result,
            Err(ProviderError::Failed { code: -1, .. })
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_signal_killed_probe_is_not_reported_as_signed_in() {
        let result = one_shot(
            "/bin/sh",
            &["-c".into(), "kill -9 $$".into()],
            "test",
            str::to_string,
        )
        .await;
        assert!(matches!(result, SignIn::Failed(_)));
    }

    #[test]
    fn only_a_real_answer_counts_as_signed_in() {
        assert!(classify(Ok("OK".into()), 60).usable());
        assert!(!classify(Ok("   ".into()), 60).usable());
        assert!(
            !classify(
                Err(ProviderError::NotFound {
                    vendor: "claude",
                    hint: String::new()
                }),
                60
            )
            .usable()
        );
    }

    /// An empty answer is its own state, not a pass and not a failure.
    #[test]
    fn an_empty_answer_is_neither_a_pass_nor_a_failure() {
        let empty = classify(Ok(String::new()), 60);
        assert_eq!(empty, SignIn::Empty);
        assert!(empty.describe("claude").contains("cannot tell"));
    }

    /// The vendor's own words survive, because they are the actionable part.
    #[test]
    fn a_failure_keeps_the_reason_the_vendor_gave() {
        let failed = classify(
            Err(ProviderError::Failed {
                vendor: "claude",
                code: 1,
                message: "please run `claude login`".into(),
            }),
            60,
        );
        assert!(
            failed.describe("claude").contains("claude login"),
            "the one part that says what to do was dropped: {}",
            failed.describe("claude")
        );
    }

    #[test]
    fn a_timeout_says_what_it_means_for_a_real_run() {
        let out = classify(
            Err(ProviderError::Process(ProcessError::Timeout {
                what: "claude".into(),
                seconds: 60,
            })),
            60,
        );
        assert_eq!(out, SignIn::TimedOut(60));
        assert!(out.describe("claude").contains("sweep would hang"));
    }
}
