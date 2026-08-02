//! One pass that grades every defect in a report against the others.
//!
//! **Given no tools at all, after measuring what happens with them.** The first
//! version got the same read-only tools a sweep gets, on the reasoning that
//! consequence is a fact about the callers rather than about the line. That
//! reasoning is still true and it lost anyway: handed thirteen defects, the
//! pass spent its whole budget reading and returned nothing — twice, at two
//! different turn limits. A grade made from the descriptions is worth a great
//! deal more than no grade at all, and the descriptions are not thin: each
//! carries a title, a location, a failure scenario and an explanation, written
//! by a reviewer who *did* read the code.

use std::path::Path;
use std::time::Duration;

use bugsleuth_domain::{SeverityVerdicts, triage_schema};

use super::{READ_ONLY_DENIED, Run, invoke};
use crate::error::ProviderError;

pub struct TriageRequest<'a> {
    pub repo: &'a Path,
    pub model: &'a str,
    pub effort: &'a str,
    /// The defect list, already formatted. Delivered on stdin like a brief: a
    /// full report easily exceeds the command-line length limit.
    pub prompt: &'a str,
    pub timeout: Duration,
    pub max_turns: u32,
    pub binary: Option<&'a str>,
    pub api_key: Option<&'a str>,
}

/// The verdicts, and whether they had to be recovered from a cut-off session.
pub struct TriageResult {
    pub verdicts: SeverityVerdicts,
    /// True when the grading model ran out of turns and was asked afterwards
    /// what it had decided. A partial grading is still worth applying, but the
    /// report must not present it as a full comparison.
    pub salvaged: bool,
}

pub async fn triage(spec: TriageRequest<'_>) -> Result<TriageResult, ProviderError> {
    let outcome = invoke(Run {
        repo: spec.repo,
        model: spec.model,
        effort: spec.effort,
        prompt: spec.prompt,
        schema: triage_schema(),
        // No tools. One turn in, one answer out.
        allowed: "",
        denied: READ_ONLY_DENIED,
        max_turns: spec.max_turns,
        timeout: spec.timeout,
        binary: spec.binary,
        api_key: spec.api_key,
        resume: None,
    })
    .await?;

    Ok(TriageResult {
        verdicts: crate::json::structured(&outcome.result)?,
        salvaged: outcome.salvaged,
    })
}
