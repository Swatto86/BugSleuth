//! What the code already says about a defect, and whether a dismissal is true.
//!
//! Split from `triage.rs` at the hard line cap, along the seam that was already
//! there: everything here reads the reviewed repository, and nothing here talks
//! to a model. A dismissal is the one verdict that removes a finding from the
//! report and from every fix prompt, so the claim behind it is checked against
//! the file rather than taken on the model's word.

use std::path::Path;

use bugsleuth_judge::Cluster;

/// The quote a verdict offers as proof the code already acknowledges this, if
/// that quote is really in the file it names.
///
/// Returns the quote and the rest of the reason. `None` when the verdict makes
/// no such claim, when the file cannot be read, or — the case that matters —
/// when the quoted words are not there. An unfounded dismissal is discarded and
/// the finding stands, which is the safe direction: the cost of being wrong is
/// a reader seeing a defect they already knew about, against a reader never
/// seeing a real one.
pub(super) fn acknowledgement(
    reason: &str,
    repo: &Path,
    cluster: &Cluster,
) -> Option<(String, String)> {
    const MARKER: &str = "ACKNOWLEDGED:";
    let rest = reason.trim().strip_prefix(MARKER)?.trim();

    // The quote is whatever the model put first, up to the end of a sentence or
    // the line. Compared with whitespace collapsed, since a comment is wrapped
    // across lines with leading slashes and the model will not reproduce that.
    let quote = rest
        .trim_start_matches(['"', '\''])
        .split(['"', '\n'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_string();
    if quote.split_whitespace().count() < 4 {
        // Too short to be evidence of anything.
        return None;
    }

    // The same bounded commentary block the prompt offered for this defect, not
    // the whole file. Searching the file accepted a comment written about an
    // unrelated defect further down, which took this one out of the actionable
    // findings and out of every fix prompt.
    let source = commentary_at(repo, cluster.representative())?;
    let flatten = |text: &str| {
        text.split_whitespace()
            .map(|word| word.trim_matches(|c: char| c == '/' || c == '*'))
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if !flatten(&source).contains(&flatten(&quote)) {
        return None;
    }
    Some((quote, rest.to_string()))
}

/// The comment block written immediately above a finding's anchor.
///
/// Read because a reviewer is not the only one who has looked at this code: the
/// person who wrote it may have already recorded why it is the way it is. Four
/// of the five findings that failed scrutiny across two self-reviews were
/// accurate observations about deliberate trade-offs documented at the exact
/// line named — and the words settling them were sitting in the file the whole
/// time, unread.
///
/// Contiguous comment lines only, and a bounded number of them: this goes into
/// a prompt, and a run of commented-out code above a defect is not commentary
/// about it.
pub(super) fn commentary_at(repo: &Path, finding: &bugsleuth_domain::Finding) -> Option<String> {
    const MOST: usize = 24;
    // The anchor crossed a JSON boundary (a resumed or supplied report), so its
    // path is untrusted until rechecked: read it only through the same lexical
    // and canonical containment gate verify_anchor uses, never a bare join.
    let path = bugsleuth_verify::checked_repo_file(repo, &finding.anchor.file).ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    // Anchors are 1-based, and the comment sits above the line itself.
    let mut index = usize::try_from(finding.anchor.line).ok()?.checked_sub(1)?;

    let mut block: Vec<&str> = Vec::new();
    while index > 0 && block.len() < MOST {
        index -= 1;
        let line = lines.get(index)?.trim();
        // Attributes and blank lines sit between a doc comment and the item it
        // documents, so they neither end the block nor join its commentary.
        if line.is_empty() || line.starts_with("#[") || line.starts_with("#![") {
            continue;
        }
        let is_comment = line.starts_with("//") || line.starts_with('#') || line.starts_with('*');
        if !is_comment {
            break;
        }
        block.push(line);
    }
    if block.is_empty() {
        return None;
    }
    block.reverse();
    Some(block.join(
        "
",
    ))
}
