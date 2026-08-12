//! Vendor selection and confinement rules for Kimi and Cursor.
//!
//! Split from `tests.rs` at the hard line cap. These assert the closed
//! `Vendor` enum's behaviour for the two adapters that share worktree
//! isolation and no schema enforcement.

use super::*;
/// Kimi is a first-class vendor with Kilo's confinement, not Claude's.
///
/// It has no tool allowlist and no read-only flag — only `--yolo`, which
/// loosens — so the one thing standing between a Kimi review and the code it
/// reviews is that it never sees the real checkout. And it takes no schema, so
/// its brief has to describe the shape in words.
#[test]
fn kimi_is_selected_by_prefix_and_must_be_isolated() {
    assert_eq!(Vendor::parse("kimi:kimi-k3"), (Vendor::Kimi, "kimi-k3"));
    assert_eq!(Vendor::parse("kimi:"), (Vendor::Kimi, ""));
    assert_eq!(resolved_label("kimi:kimi-k3"), "kimi:kimi-k3");

    assert!(
        Vendor::Kimi.needs_isolation(),
        "a Kimi sweep pointed at the real checkout has nothing stopping it writing"
    );
    assert!(
        !Vendor::Kimi.enforces_schema(),
        "Kimi takes no schema, so its brief must describe the shape instead"
    );
    // The known-present control: Claude differs on both counts, so this is not
    // asserting something true of every vendor.
    assert!(!Vendor::Claude.needs_isolation());
    assert!(Vendor::Claude.enforces_schema());
}

/// A Kimi model in the plan makes Kimi one of the vendors to pre-check.
#[test]
fn a_kimi_model_selects_kimi_for_the_precheck() {
    let models = vec!["sonnet".to_string(), "kimi:kimi-k3".to_string()];
    assert_eq!(
        precheck::vendors_for(&models),
        vec![Vendor::Claude, Vendor::Kimi]
    );
}

/// Cursor is a first-class vendor: ask-mode is read-only, but sweeps still
/// isolate because there is no ignore-rules flag for project instructions.
#[test]
fn cursor_is_selected_by_prefix_and_must_be_isolated() {
    assert_eq!(
        Vendor::parse("cursor:composer-2.5"),
        (Vendor::Cursor, "composer-2.5")
    );
    assert_eq!(Vendor::parse("cursor:"), (Vendor::Cursor, ""));
    assert_eq!(resolved_label("cursor:composer-2.5"), "cursor:composer-2.5");

    assert!(
        Vendor::Cursor.needs_isolation(),
        "without ignore-rules, a Cursor sweep must not see the real checkout's instructions"
    );
    assert!(
        !Vendor::Cursor.enforces_schema(),
        "Cursor takes no schema, so its brief must describe the shape instead"
    );
    assert!(!Vendor::Claude.needs_isolation());
    assert!(Vendor::Claude.enforces_schema());
}

#[test]
fn a_cursor_model_selects_cursor_for_the_precheck() {
    let models = vec!["sonnet".to_string(), "cursor:composer-2.5".to_string()];
    assert_eq!(
        precheck::vendors_for(&models),
        vec![Vendor::Claude, Vendor::Cursor]
    );
}
