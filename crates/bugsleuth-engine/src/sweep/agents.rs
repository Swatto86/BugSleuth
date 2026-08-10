use super::Vendor;

/// Whether this vendor can be asked to delegate, and what to tell it.
///
/// `Ok` carries the instruction added to the brief; `Err` carries the reason,
/// in that vendor's own terms — Kilo has a read-only Ask agent that cannot
/// delegate, while Kimi simply has no subagent mode to ask for. One match, so a
/// vendor cannot end up with neither an instruction nor an explanation, and so
/// the answer cannot drift between the pre-run refusal and the sweep.
///
/// This used to be duplicated: `plan.rs` refused `vendor == "kilo"` by name,
/// the frontend's `supportsAgents` said `!== "kilo"`, and the sweep asked here.
/// Adding Kimi therefore made the UI offer a checkbox that turned every lane
/// into NOT SWEPT — a whole run producing nothing, which is the failure this
/// project exists to prevent rather than cause.
pub(crate) fn support(vendor: Vendor, _model: &str) -> Result<&'static str, &'static str> {
    match vendor {
        Vendor::Claude => Ok(
            "Delegate to exactly two foreground Explore subagents in parallel, dividing this lane into independent search areas. Do not use Workflow, background agents, or delayed wakeups. Wait for both subagents in this turn, then verify and synthesize their evidence into your one required JSON response. Keep both subagents read-only and inside this mandate. If delegation is unavailable, continue alone.",
        ),
        Vendor::Codex => Ok(
            "Use multiple Codex subagents in parallel, dividing this lane into independent search areas. Keep every subagent read-only and inside this mandate, then verify and synthesize their evidence into your one required JSON response. If delegation is unavailable, continue alone.",
        ),
        Vendor::Kilo => Err("Kilo's read-only Ask agent cannot delegate"),
        Vendor::Kimi => Err("Kimi has no subagent mode BugSleuth can ask for"),
    }
}

/// The vendors that cannot delegate, by label.
///
/// Derived from [`support`] rather than written out, so it cannot disagree with
/// it. Read by the cross-language check that keeps the frontend's own list of
/// agent-capable vendors in step — the two sides of that boundary are strings
/// and no compiler spans them.
#[must_use]
pub fn cannot_delegate() -> Vec<&'static str> {
    [Vendor::Claude, Vendor::Codex, Vendor::Kilo, Vendor::Kimi]
        .into_iter()
        .filter(|vendor| support(*vendor, "").is_err())
        .map(Vendor::label)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vendor answers, and the two halves cannot both be missing.
    #[test]
    fn every_vendor_either_delegates_or_says_why_not() {
        for vendor in [Vendor::Claude, Vendor::Codex, Vendor::Kilo, Vendor::Kimi] {
            match support(vendor, "") {
                Ok(instruction) => assert!(
                    instruction.contains("subagent") || instruction.contains("Explore"),
                    "{} was given an instruction that asks for nothing",
                    vendor.label()
                ),
                Err(reason) => assert!(
                    reason.len() > 20,
                    "{} refuses agents with no usable reason",
                    vendor.label()
                ),
            }
        }
    }

    /// The derived list is the list, and it is not empty or everything.
    #[test]
    fn the_refusing_vendors_are_derived_from_the_same_answer() {
        let refusing = cannot_delegate();
        assert_eq!(refusing, ["kilo", "kimi"], "{refusing:?}");
        assert!(support(Vendor::Claude, "").is_ok());
        assert!(support(Vendor::Codex, "").is_ok());
    }
}
