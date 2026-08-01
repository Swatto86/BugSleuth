//! Reading `kilo models --verbose`.
//!
//! The output is a sequence of records: a bare `provider/model` line, then that
//! model's JSON. Two things in that JSON cannot be worked out any other way:
//!
//! - **`hasUserByokAvailable`** — whether the model bills to a plan you bought
//!   from the provider rather than to Kilo Gateway credit. A handful of `kilo/`
//!   models are BYOK and look exactly like gateway models otherwise.
//! - **`variants`** — the reasoning efforts *that model* accepts. They are not
//!   uniform and not always a graded scale: most offer nothing at all, many
//!   offer `low, medium, high`, and some offer `instant, thinking`, which is a
//!   pair rather than a ladder.
//!
//! Both are read here rather than guessed from the id, because a guess about
//! either one produces a control that lies: the wrong account, or an effort the
//! provider will reject.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One model as the catalogue describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) id: String,
    /// Billed to your own plan with the provider, not to Kilo Gateway.
    pub(super) byok: bool,
    /// Reasoning efforts this model accepts, weakest first. Empty means it
    /// takes none, which the UI must show as unavailable rather than guess at.
    pub(super) efforts: Vec<String>,
}

/// The fields we read out of a record. Everything else is ignored, so the
/// catalogue can grow without this needing to know.
#[derive(Deserialize, Default)]
struct Record {
    #[serde(default, rename = "hasUserByokAvailable")]
    has_user_byok_available: bool,
    /// Keys are the effort names; the values describe what each one does, which
    /// we do not need.
    #[serde(default)]
    variants: BTreeMap<String, serde_json::Value>,
}

/// Parse the whole listing into one entry per model.
///
/// A record whose JSON will not parse yields an entry with no extras rather
/// than being dropped: the model is still selectable, it just cannot offer an
/// effort. Losing a model from the menu would be the worse failure.
pub(super) fn parse(listing: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut block = String::new();

    for line in listing.lines() {
        if is_model_id(line) {
            finish(&mut entries, &block);
            block.clear();
            entries.push(Entry {
                id: line.trim().to_string(),
                byok: false,
                efforts: Vec::new(),
            });
        } else if !entries.is_empty() {
            block.push_str(line);
            block.push('\n');
        }
    }
    finish(&mut entries, &block);
    entries
}

/// Attach a finished JSON block to the entry it belongs to.
fn finish(entries: &mut [Entry], block: &str) {
    let Some(entry) = entries.last_mut() else {
        return;
    };
    let Ok(record) = serde_json::from_str::<Record>(block.trim()) else {
        return;
    };
    entry.byok = record.has_user_byok_available;
    entry.efforts = ordered(record.variants.into_keys());
}

/// Whether a line is a bare `provider/model` id rather than JSON or a banner.
///
/// The CLI prints ASCII art before the list and pretty-printed JSON after each
/// id, so this has to reject both. An id sits at the left margin, contains a
/// slash, and carries none of the punctuation JSON is made of.
fn is_model_id(line: &str) -> bool {
    !line.starts_with(char::is_whitespace)
        && !line.trim().is_empty()
        && line.contains('/')
        && !line.contains(' ')
        && !line.contains('"')
        && !line.contains('{')
        && !line.contains('}')
}

/// Effort names in the order a person would expect to see them.
///
/// Sorting is a display concern only — whatever is chosen is passed to Kilo
/// exactly as the catalogue spelled it. It matters because the catalogue does
/// not agree with itself on order: most models list `none, low, medium, high`
/// ascending, and a few list `max, high, low` descending. Showing one of those
/// backwards reads as a bug.
///
/// Names not in the table sort after the ones that are, alphabetically, which
/// is at least stable. `instant` and `thinking` are in it because they are a
/// real pair Kilo uses — off and on rather than a ladder — and leaving them
/// unranked would put them in the wrong place by accident.
const RANKED: [&str; 10] = [
    "none", "minimal", "instant", "low", "medium", "thinking", "high", "xhigh", "ultra", "max",
];

fn ordered(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = names.collect();
    names.sort_by_key(|name| {
        (
            RANKED
                .iter()
                .position(|r| *r == name)
                .unwrap_or(RANKED.len()),
            name.clone(),
        )
    });
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Abridged from real `kilo models --verbose` output, with the shape — id
    /// line, then JSON — preserved exactly.
    const LISTING: &str = r#"
██  ██ ████
~~  ~~ ~~~~

kilo/ai21/jamba-large-1.7
{
  "id": "ai21/jamba-large-1.7",
  "cost": { "input": 2, "output": 8 },
  "variants": {},
  "hasUserByokAvailable": false
}
kilo/zai-coding/glm-5.2
{
  "id": "zai-coding/glm-5.2",
  "variants": {
    "high": { "reasoning": { "effort": "high" } },
    "max": { "reasoning": { "effort": "max" } }
  },
  "hasUserByokAvailable": true
}
openrouter/z-ai/glm-4.6
{
  "id": "z-ai/glm-4.6",
  "variants": {
    "medium": {},
    "low": {},
    "high": {}
  }
}
"#;

    #[test]
    fn each_model_gets_its_own_efforts_rather_than_the_vendors() {
        // The reason this file exists. A hardcoded low/medium/high would offer
        // two levels glm-5.2 rejects and hide the one it actually adds.
        let entries = parse(LISTING);
        let by_id = |id: &str| {
            entries
                .iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| panic!("{id} missing from {:?}", entries.len()))
                .clone()
        };

        assert_eq!(by_id("kilo/zai-coding/glm-5.2").efforts, ["high", "max"]);
        assert_eq!(
            by_id("openrouter/z-ai/glm-4.6").efforts,
            ["low", "medium", "high"],
            "efforts must come out weakest-first, not alphabetically"
        );
        assert!(
            by_id("kilo/ai21/jamba-large-1.7").efforts.is_empty(),
            "an empty variants block means the model takes no effort setting"
        );
    }

    #[test]
    fn the_byok_flag_is_read_per_record_and_does_not_leak() {
        // The catalogue is alphabetical, so a flag leaking forwards would
        // mislabel most of it — and mislabelling says the wrong thing about
        // which account a sweep spends from.
        let entries = parse(LISTING);
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.byok)
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["kilo/zai-coding/glm-5.2"]
        );
    }

    #[test]
    fn the_banner_is_not_mistaken_for_a_model() {
        let entries = parse(LISTING);
        assert_eq!(entries.len(), 3, "got {:?}", entries);
    }

    #[test]
    fn a_record_whose_json_is_unreadable_keeps_its_model_selectable() {
        // Dropping the model would be the worse failure: it would vanish from
        // the menu with no explanation, and it is still perfectly usable.
        let entries = parse("kilo/x/y\n{ this is not json\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "kilo/x/y");
        assert!(entries[0].efforts.is_empty());
        assert!(!entries[0].byok);
    }

    #[test]
    fn unknown_effort_names_sort_after_known_ones_rather_than_at_random() {
        let ordered = ordered(
            ["high", "zebra", "low", "apple"]
                .into_iter()
                .map(str::to_string),
        );
        assert_eq!(ordered, ["low", "high", "apple", "zebra"]);
    }
}
