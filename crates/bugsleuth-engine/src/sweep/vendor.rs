//! Which coding CLI a sweep or apply runs.
//!
//! Split from `sweep.rs` at the hard line cap. The closed enum is the whole
//! of BugSleuth's vendor surface: parsing a `vendor:model` spec, labelling a
//! report, and deciding whether a sweep may touch the real checkout.

/// Which CLI to run, and which model within it.
///
/// A plain enum because the supported provider set is closed and small.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Claude,
    Codex,
    Kilo,
    /// Moonshot's Kimi Code CLI. Present because a Kimi subscription reaches
    /// models a bring-your-own-key route does not, and only the native CLI can
    /// use that session.
    Kimi,
    /// Cursor Agent CLI (`agent` on PATH). Ask mode is read-only per
    /// invocation; sweeps still use a worktree because there is no
    /// ignore-rules flag.
    Cursor,
}

impl Vendor {
    /// Read a `vendor:model` spec such as `codex:gpt-5.6-codex`. A bare name
    /// means Claude, which keeps the common case short.
    pub fn parse(raw: &str) -> (Vendor, &str) {
        let spec = raw.trim();
        match spec.split_once(':') {
            Some(("codex", model)) => (Vendor::Codex, model),
            Some(("claude", model)) => (Vendor::Claude, model),
            Some(("kilo", model)) => (Vendor::Kilo, model),
            Some(("kimi", model)) => (Vendor::Kimi, model),
            Some(("cursor", model)) => (Vendor::Cursor, model),
            _ => (Vendor::Claude, spec),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Vendor::Claude => "claude",
            Vendor::Codex => "codex",
            Vendor::Kilo => "kilo",
            Vendor::Kimi => "kimi",
            Vendor::Cursor => "cursor",
        }
    }

    /// Whether the CLI can be handed a JSON Schema it will actually enforce.
    /// Kilo, Kimi and Cursor cannot, so their briefs describe the shape in words.
    pub fn enforces_schema(self) -> bool {
        !matches!(self, Vendor::Kilo | Vendor::Kimi | Vendor::Cursor)
    }

    /// Whether a sweep by this vendor must run in a throwaway checkout rather
    /// than against the repository itself.
    ///
    /// Claude takes a tool allowlist and Codex takes `--sandbox read-only`, so
    /// neither needs a copy. Kilo and Kimi have no per-invocation write boundary
    /// BugSleuth can trust without isolating. Cursor has `--mode ask` (read-only)
    /// but no ignore-rules flag, so project instructions would otherwise brief
    /// the reviewer — isolation strips those from a throwaway tree.
    pub fn needs_isolation(self) -> bool {
        matches!(self, Vendor::Kilo | Vendor::Kimi | Vendor::Cursor)
    }
}

/// The `vendor:model` a spec resolves to, exactly as a report records it.
///
/// One function, because the label was being built in one place and compared
/// against a raw config string in another. A unit configured as `sonnet`
/// produced a report saying `claude:sonnet`, and the equality test between them
/// was never true — so a cancelled run counted every finished sweep as still
/// outstanding and told the reader that lanes it had already swept were not
/// reached.
#[must_use]
pub fn resolved_label(spec: &str) -> String {
    let (vendor, model) = Vendor::parse(spec);
    format!("{}:{model}", vendor.label())
}
