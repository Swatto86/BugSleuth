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
    #[error("`{0}` resolves to a location outside the repository")]
    ResolvesOutsideRepo(String),
}

/// A repository file at `raw`, checked to exist and stay inside `repo`.
///
/// The lexical and canonical containment checks [`verify_anchor`] applies to a
/// model's claim, factored out so a path that has crossed a JSON boundary — a
/// resumed or externally-supplied report — can be revalidated before it is read.
/// `Finding` and `LaneReport` both derive `Deserialize`, so a cached report can
/// carry an arbitrary anchor path; reading it unchecked would let one escape
/// the repository.
pub fn checked_repo_file(repo: &Path, raw: &str) -> Result<PathBuf, Rejection> {
    let relative = safe_relative_path(raw)?;
    let absolute = repo.join(&relative);
    if !absolute.is_file() {
        return Err(Rejection::NoSuchFile(raw.to_string()));
    }
    // The component check above is lexical: it runs before the path touches the
    // disk, so it says nothing about where the path *resolves*. A reviewed
    // repository is untrusted input, and one containing a symlink at an
    // innocent-looking path — `src/util.rs` pointing at a private key or at the
    // user's own settings — would otherwise have its target read and quoted
    // back into a report that gets copied into a prompt.
    if !resolves_inside(repo, &absolute) {
        return Err(Rejection::ResolvesOutsideRepo(raw.to_string()));
    }
    Ok(absolute)
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
    let absolute = checked_repo_file(repo, &raw.file)?;

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
    // shows what the code actually says. The end comes from where the match
    // actually finished, not from the needle's length: a match that skipped an
    // interior blank line spans more physical lines than the needle has, and
    // `found + needle.len()` would stop short and drop its last line(s).
    let end = matches_at(&trimmed, &needle, found)
        .unwrap_or(found + needle.len())
        .min(haystack.len());
    let snippet = haystack[found..end].join("\n");

    Ok(VerifiedAnchor {
        file: absolute
            .strip_prefix(repo)
            .unwrap_or(&absolute)
            .to_string_lossy()
            .replace('\\', "/"),
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
        .filter(|start| matches_at(file_lines, needle, *start).is_some())
        .min_by_key(|start| start.abs_diff(claimed_index))
}

/// Where a match of `needle` starting at `start` finishes, or `None`.
///
/// Returns the cursor one past the last matched file line, so the caller can
/// re-quote the region that was actually matched. This matters because blank
/// lines *between* needle lines are skipped, so the matched region can span more
/// physical lines than the needle has — and re-quoting `needle.len()` lines then
/// stops short, dropping the final line(s) the finding points at.
///
/// The first needle line must match `start` itself — a match may not begin by
/// skipping forward over blank lines, or every blank line preceding a match
/// would also count as a match and the reported line would drift backwards.
fn matches_at(file_lines: &[&str], needle: &[&str], start: usize) -> Option<usize> {
    let mut cursor = start;
    for (index, wanted) in needle.iter().enumerate() {
        if index > 0 {
            while file_lines.get(cursor).is_some_and(|line| line.is_empty()) {
                cursor += 1;
            }
        }
        match file_lines.get(cursor) {
            Some(line) if line == wanted => cursor += 1,
            _ => return None,
        }
    }
    Some(cursor)
}

/// Whether `candidate` is genuinely inside `repo` once symlinks are followed.
///
/// Both sides are canonicalised, because comparing a resolved path against an
/// unresolved root is its own false negative: on Windows a temp directory is
/// commonly reached through a symlinked user path, and on macOS `/tmp` is a
/// symlink to `/private/tmp`, so the repository root itself resolves elsewhere.
///
/// A root that cannot be canonicalised means the repository is gone, and a
/// candidate that cannot be canonicalised means the file vanished between the
/// existence check and here. Both are refusals: this function only ever answers
/// "yes, provably inside".
fn resolves_inside(repo: &Path, candidate: &Path) -> bool {
    match (repo.canonicalize(), candidate.canonicalize()) {
        (Ok(root), Ok(target)) => target.starts_with(root),
        _ => false,
    }
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
#[path = "anchor/tests.rs"]
mod tests;
