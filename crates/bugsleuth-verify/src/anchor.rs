//! Checking that a finding quotes code that really exists.

use std::path::{Component, Path, PathBuf};

use bugsleuth_domain::{RawFinding, VerifiedAnchor};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("the path `{0}` points outside the repository")]
    PathEscapesRepo(String),
    #[error("no file named `{0}` exists in the repository")]
    NoSuchFile(String),
    #[error("`{0}` could not be read as text: {1}")]
    Unreadable(String, String),
    #[error("the finding quoted no code")]
    EmptySnippet,
    #[error("the quoted code does not appear anywhere in `{0}`")]
    SnippetNotInFile(String),
}

/// Confirm that `raw`'s snippet appears in the file it names, and report where.
///
/// Deliberately *not* an exact `file:line` equality check. Models reliably quote
/// real code but routinely misnumber it by a few lines, and discarding a genuine
/// defect over an off-by-three would throw away most of the value this filter is
/// meant to protect. So the snippet must be found verbatim somewhere in the
/// file — that is what kills hallucinations — and the line number is then
/// corrected to where it actually is. The correction stays visible in the
/// report via [`VerifiedAnchor::was_corrected`].
///
/// Comparison ignores leading and trailing whitespace per line, because a model
/// re-indents quoted code far more often than it invents it.
pub fn verify_anchor(repo: &Path, raw: &RawFinding) -> Result<VerifiedAnchor, Rejection> {
    let relative = safe_relative_path(&raw.file)?;
    let absolute = repo.join(&relative);
    if !absolute.is_file() {
        return Err(Rejection::NoSuchFile(raw.file.clone()));
    }

    let contents = std::fs::read_to_string(&absolute)
        .map_err(|e| Rejection::Unreadable(raw.file.clone(), e.to_string()))?;

    let needle: Vec<&str> = raw
        .snippet
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if needle.is_empty() {
        return Err(Rejection::EmptySnippet);
    }

    let haystack: Vec<&str> = contents.lines().collect();
    let trimmed: Vec<&str> = haystack.iter().map(|line| line.trim()).collect();

    let found = find_match(&trimmed, &needle, raw.line)
        .ok_or_else(|| Rejection::SnippetNotInFile(raw.file.clone()))?;

    // Re-quote from the file rather than echoing the model's copy, so the report
    // shows what the code actually says.
    let end = (found + needle.len()).min(haystack.len());
    let snippet = haystack[found..end].join("\n");

    Ok(VerifiedAnchor {
        file: relative.to_string_lossy().replace('\\', "/"),
        line: u32::try_from(found + 1).unwrap_or(u32::MAX),
        claimed_line: raw.line,
        snippet,
    })
}

/// Index of the first line of the best match, or `None`.
///
/// The match *closest* to the claimed line wins, so a snippet that legitimately
/// occurs several times in a file (a repeated guard, an identical assignment in
/// two functions) anchors to the occurrence the model actually meant rather than
/// to whichever one happens to come first.
fn find_match(file_lines: &[&str], needle: &[&str], claimed_line: u32) -> Option<usize> {
    if needle.len() > file_lines.len() {
        return None;
    }
    let claimed_index = claimed_line.saturating_sub(1) as usize;
    (0..file_lines.len())
        .filter(|start| matches_at(file_lines, needle, *start))
        .min_by_key(|start| start.abs_diff(claimed_index))
}

/// Whether `needle` matches starting exactly at `start`.
///
/// The first needle line must match `start` itself — a match may not begin by
/// skipping forward over blank lines, or every blank line preceding a match
/// would also count as a match and the reported line would drift backwards.
/// Blank lines *between* needle lines are skipped, so a quote that dropped an
/// interior blank line still matches.
fn matches_at(file_lines: &[&str], needle: &[&str], start: usize) -> bool {
    let mut cursor = start;
    for (index, wanted) in needle.iter().enumerate() {
        if index > 0 {
            while file_lines.get(cursor).is_some_and(|line| line.is_empty()) {
                cursor += 1;
            }
        }
        match file_lines.get(cursor) {
            Some(line) if line == wanted => cursor += 1,
            _ => return false,
        }
    }
    true
}

/// Reject anything that would read outside the repository.
///
/// The path comes from a model, which makes it untrusted input crossing into
/// filesystem access. `..` and absolute paths are refused outright rather than
/// normalised, because a finding that needs to escape the repository is not a
/// finding about the repository.
fn safe_relative_path(raw: &str) -> Result<PathBuf, Rejection> {
    let candidate = Path::new(raw.trim().trim_start_matches("./"));
    let mut out = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Rejection::PathEscapesRepo(raw.to_string()));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Rejection::PathEscapesRepo(raw.to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bugsleuth_domain::Severity;

    fn raw(file: &str, line: u32, snippet: &str) -> RawFinding {
        RawFinding {
            title: "t".into(),
            severity: Severity::High,
            file: file.into(),
            line,
            snippet: snippet.into(),
            explanation: "e".into(),
            failure_scenario: "f".into(),
            fix: Default::default(),
        }
    }

    /// A throwaway repository. Each test gets its own directory: these tests run
    /// in parallel in one process, and a shared fixture path meant one test
    /// could read the file while another was midway through rewriting it.
    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("bugsleuth-anchor-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("src"));
        let _ = std::fs::write(
            dir.join("src/lib.rs"),
            "fn one() {\n    let x = 1;\n\n    let y = 2;\n}\n\nfn two() {\n    let x = 1;\n}\n",
        );
        dir
    }

    #[test]
    fn accepts_a_snippet_that_really_is_at_the_claimed_line() {
        let repo = fixture("accepts_a_snippet_that_really_is_at_the_claimed_line");
        let anchor = verify_anchor(&repo, &raw("src/lib.rs", 2, "let x = 1;"));
        let anchor = anchor.unwrap_or_else(|e| panic!("rejected a true anchor: {e}"));
        assert_eq!(anchor.line, 2);
        assert!(!anchor.was_corrected());
    }

    #[test]
    fn corrects_a_line_number_the_model_got_wrong_instead_of_discarding_the_finding() {
        let repo =
            fixture("corrects_a_line_number_the_model_got_wrong_instead_of_discarding_the_finding");
        let anchor = verify_anchor(&repo, &raw("src/lib.rs", 5, "let y = 2;"));
        let anchor = anchor.unwrap_or_else(|e| panic!("rejected a near-miss anchor: {e}"));
        assert_eq!(anchor.line, 4);
        assert_eq!(anchor.claimed_line, 5);
        assert!(anchor.was_corrected());
    }

    #[test]
    fn rejects_code_that_does_not_exist_in_the_file() {
        let repo = fixture("rejects_code_that_does_not_exist_in_the_file");
        let result = verify_anchor(&repo, &raw("src/lib.rs", 2, "let invented = true;"));
        assert_eq!(
            result.err(),
            Some(Rejection::SnippetNotInFile("src/lib.rs".into()))
        );
    }

    #[test]
    fn rejects_a_file_that_does_not_exist() {
        let repo = fixture("rejects_a_file_that_does_not_exist");
        let result = verify_anchor(&repo, &raw("src/imaginary.rs", 1, "let x = 1;"));
        assert!(matches!(result, Err(Rejection::NoSuchFile(_))));
    }

    #[test]
    fn rejects_a_path_that_climbs_out_of_the_repository() {
        let repo = fixture("rejects_a_path_that_climbs_out_of_the_repository");
        let result = verify_anchor(&repo, &raw("../../../etc/passwd", 1, "root"));
        assert!(matches!(result, Err(Rejection::PathEscapesRepo(_))));
    }

    #[test]
    fn tolerates_re_indented_code() {
        let repo = fixture("tolerates_re_indented_code");
        let anchor = verify_anchor(&repo, &raw("src/lib.rs", 2, "        let x = 1;   "));
        assert!(
            anchor.is_ok(),
            "re-indented snippet was rejected: {anchor:?}"
        );
    }

    #[test]
    fn matches_a_multi_line_quote_across_a_blank_line() {
        let repo = fixture("matches_a_multi_line_quote_across_a_blank_line");
        let anchor = verify_anchor(&repo, &raw("src/lib.rs", 2, "let x = 1;\nlet y = 2;"));
        let anchor = anchor.unwrap_or_else(|e| panic!("rejected a multi-line anchor: {e}"));
        assert_eq!(anchor.line, 2);
    }

    #[test]
    fn a_snippet_occurring_twice_anchors_to_the_occurrence_nearest_the_claim() {
        let repo = fixture("a_snippet_occurring_twice_anchors_to_the_occurrence_nearest_the_claim");
        let far = verify_anchor(&repo, &raw("src/lib.rs", 8, "let x = 1;"));
        // Both line 2 and line 8 hold `let x = 1;`; the claim points at the second.
        assert_eq!(far.map(|a| a.line).unwrap_or(0), 8);
    }
}
