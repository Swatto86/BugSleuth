use super::Vendor;

pub(super) fn instruction(vendor: Vendor, _model: &str) -> Option<&'static str> {
    match vendor {
        Vendor::Claude => Some(
            "Delegate to exactly two foreground Explore subagents in parallel, dividing this lane into independent search areas. Do not use Workflow, background agents, or delayed wakeups. Wait for both subagents in this turn, then verify and synthesize their evidence into your one required JSON response. Keep both subagents read-only and inside this mandate. If delegation is unavailable, continue alone.",
        ),
        Vendor::Codex => Some(
            "Use multiple Codex subagents in parallel, dividing this lane into independent search areas. Keep every subagent read-only and inside this mandate, then verify and synthesize their evidence into your one required JSON response. If delegation is unavailable, continue alone.",
        ),
        Vendor::Kilo => None,
    }
}
