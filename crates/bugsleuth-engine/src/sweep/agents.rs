use super::Vendor;

pub(super) fn instruction(vendor: Vendor, model: &str) -> Option<&'static str> {
    match vendor {
        Vendor::Claude if bugsleuth_provider::models::supports_ultracode(model) => Some(
            "Use one Ultracode dynamic workflow containing exactly two agent() calls total, launched in parallel in one phase and divided into independent search areas. Do not create verifier agents or another workflow: after the workflow returns, verify and synthesize its evidence yourself into the one required JSON response. Keep both agents read-only and inside this mandate. If workflows are unavailable, delegate to two parallel Explore subagents instead; if delegation is unavailable, continue alone.",
        ),
        Vendor::Claude => Some(
            "Delegate to two parallel Explore subagents, dividing this lane into independent search areas. Keep every subagent read-only and inside this mandate, then verify and synthesize their evidence into your one required JSON response. If delegation is unavailable, continue alone.",
        ),
        Vendor::Codex => Some(
            "Use multiple Codex subagents in parallel, dividing this lane into independent search areas. Keep every subagent read-only and inside this mandate, then verify and synthesize their evidence into your one required JSON response. If delegation is unavailable, continue alone.",
        ),
        Vendor::Kilo => None,
    }
}
