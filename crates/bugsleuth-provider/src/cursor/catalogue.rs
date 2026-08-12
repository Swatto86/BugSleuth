//! Parsing `agent models` / `--list-models` output into the picker catalogue.

/// One model id from the CLI's listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub id: String,
}

/// Parse the human listing Cursor prints.
///
/// Lines look like `composer-2.5 - Composer 2.5` or
/// `auto - Auto (current, default)`. Anything else is ignored, so a header or
/// a blank line cannot empty the menu.
pub(super) fn parse(listing: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in listing.lines() {
        let line = line.trim();
        if line.is_empty() || line.eq_ignore_ascii_case("available models") {
            continue;
        }
        let Some((id, _label)) = line.split_once(" - ") else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || id.contains(' ') {
            continue;
        }
        out.push(Entry { id: id.to_string() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_listing_shape() {
        let entries = parse(
            "Available models\n\nauto - Auto (current, default)\ncomposer-2.5 - Composer 2.5\n",
        );
        assert_eq!(
            entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["auto", "composer-2.5"]
        );
    }

    #[test]
    fn ignores_noise_rather_than_emptying_the_menu() {
        assert!(parse("Available models\n\n(no models)\n").is_empty());
        assert_eq!(parse("composer-2.5 - Composer 2.5").len(), 1);
    }
}
