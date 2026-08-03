//! How much text a repository will put in front of a model.
//!
//! A sweep fails on context long after the quota for it is spent, and the
//! message it fails with names the model rather than the cause. On a 3,500-line
//! Rust project a Kilo sweep died with "Compaction exhausted: context still
//! exceeds model limits after 3 attempts" — the code was never the problem. The
//! repository carried a 108 KB teaching book, so its markdown outweighed its
//! source, and the model read the book.
//!
//! What this deliberately does **not** do is predict whether a given model will
//! fit. Context windows are a property of the model *and* the subscription
//! routed to it — the same Kilo model id has a different limit depending on
//! whose account is behind it — and no vendor CLI here reports one. A table of
//! guessed limits would be wrong the moment a tier changed, and wrong silently
//! while quoting a confident number.
//!
//! So this measures what is actually there and names the files responsible.
//! The reader knows their own limits; they cannot know, without being told,
//! that a documentation file is larger than their entire codebase.

use std::path::Path;

/// A single non-code file at least this large is worth naming on its own. Set
/// where it is because a file this size is no longer incidental: it is a
/// deliberate document, and the reader can decide whether a code review should
/// be reading it.
const NOTABLE: u64 = 64 * 1024;

/// How many oversized files to name. Enough to show a pattern, few enough that
/// the warning stays readable.
const NAMED: usize = 3;

/// Directories never worth measuring: build output and dependencies are not
/// what a reviewer is pointed at, and `target` alone can dwarf everything else.
const SKIP: [&str; 6] = [".git", "target", "node_modules", "dist", "build", ".venv"];

/// Generated files that are large, are not read by anyone reviewing code, and
/// cannot be acted on. The first run of this check named `Cargo.lock` at 106 KB
/// beside the file that actually broke a sweep, which is how a warning teaches
/// people to skim past it.
const GENERATED: [&str; 7] = [
    "cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "gemfile.lock",
    "composer.lock",
];

/// Extensions treated as source. Not exhaustive, and does not need to be — a
/// language missing from this list lands in `other`, which at worst produces a
/// warning naming a file the reader recognises as code and ignores.
const CODE: [&str; 24] = [
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "c", "h", "cpp", "hpp", "cs", "rb", "php",
    "swift", "kt", "sh", "ps1", "psm1", "sql", "css", "html", "vue",
];

/// What a repository holds, split by whether a reviewer is there to read it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bulk {
    pub code: u64,
    pub other: u64,
    /// Non-code files of at least `NOTABLE`, largest first, as (path, bytes).
    pub heavy: Vec<(String, u64)>,
}

impl Bulk {
    /// Whether anything here is worth saying before quota is spent.
    ///
    /// Two triggers, because they catch different shapes. A single very large
    /// document is the case that broke a real sweep. Non-code text outweighing
    /// source is the case where no one file stands out but the total does.
    pub fn worth_saying(&self) -> bool {
        !self.heavy.is_empty() || (self.other > self.code && self.code > 0)
    }
}

/// Measure a repository.
///
/// Unreadable entries are skipped rather than failing the run: this informs a
/// warning, and a warning that can abort a review is worse than no warning.
pub fn measure(repo: &Path) -> Bulk {
    let mut bulk = Bulk::default();
    walk(repo, repo, &mut bulk);
    bulk.heavy.sort_by_key(|b| std::cmp::Reverse(b.1));
    bulk.heavy.truncate(NAMED);
    bulk
}

fn walk(root: &Path, dir: &Path, bulk: &mut Bulk) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if path.is_dir() {
            if !SKIP.contains(&name.as_ref()) && !name.starts_with('.') {
                walk(root, &path, bulk);
            }
            continue;
        }

        if GENERATED.contains(&name.to_lowercase().as_str()) {
            continue;
        }

        let Ok(meta) = entry.metadata() else { continue };
        let size = meta.len();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if CODE.contains(&extension.as_str()) {
            bulk.code += size;
        } else {
            bulk.other += size;
            if size >= NOTABLE {
                let shown = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                bulk.heavy.push((shown, size));
            }
        }
    }
}

/// The warning, or nothing.
///
/// Names the files rather than describing the problem in the abstract, because
/// the action available to the reader is per file: scope the review away from
/// it, or route this model somewhere with room for it.
pub fn caution(bulk: &Bulk) -> Option<String> {
    if !bulk.worth_saying() {
        return None;
    }

    let mut lines = vec![format!(
        "this repository holds {} of source and {} of everything else.",
        kb(bulk.code),
        kb(bulk.other)
    )];

    for (path, size) in &bulk.heavy {
        lines.push(format!("           {} is {}", path, kb(*size)));
    }

    lines.push(
        "           A model reads what is in front of it. If a sweep fails on context,\n           \
         that is the first thing to scope out (--scope) or to route to a model with\n           \
         more room — the size of the code is rarely the cause."
            .to_string(),
    );

    Some(lines.join("\n"))
}

fn kb(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

#[cfg(test)]
#[path = "bulk/tests.rs"]
mod tests;
