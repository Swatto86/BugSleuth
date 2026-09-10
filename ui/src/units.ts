/**
 * What a configuration costs to run.
 *
 * Split from `model.ts` at the hard line cap, along the seam already there:
 * these are the numbers the window puts in front of someone before they commit
 * to paying for a run — how many sweeps, and how many rounds those take. Every
 * one of them mirrors something in `plan.rs`, and has to keep mirroring it: an
 * estimate that disagrees with the engine is worse than no estimate, because it
 * is believed.
 */

import {
  LANES,
  MAX_PASSES,
  type ModelSetting,
  splitId,
  vendorOf,
} from "./model.ts";

/**
 * How many times a model sweeps each lane, tolerating settings written before
 * passes existed — those have no field at all, and must read as one rather than
 * turning the whole sweep estimate into NaN.
 */
function passesOf(model: ModelSetting): number {
  return Math.max(1, model.passes ?? 1);
}

/**
 * The pass counts a row's selector should offer: the usual 1–3, plus whatever
 * value is actually stored if it falls outside them.
 *
 * Rust deserializes `passes` as an unrestricted `usize` but caps it at
 * `MAX_PASSES` (25) and refuses the run above it. A stored value above the cap
 * is therefore not a valid backend instruction; clamping it to the cap keeps the
 * control honest and lets the user choose a runnable count.
 */
export function passChoices(passes: number | undefined): number[] {
  const chosen = Math.max(1, passes ?? 1);
  const capped = Math.min(MAX_PASSES, chosen);
  const choices = [1, 2, 3];
  if (!choices.includes(capped)) choices.push(capped);
  return choices.sort((left, right) => left - right);
}

/**
 * Every (model, lane, effort, agent mode, pass) unit this configuration implies.
 *
 * Mirrors plan.rs exactly, including its dedup: the engine enumerates these
 * tuples and drops exact duplicates, so a model listed twice against one lane
 * runs max(passes), not the sum. Summing per row here showed "4 sweeps" for a
 * run the engine executes as 3 — the pre-run estimate is only worth showing if
 * it counts what will actually run.
 */
/**
 * The canonical spelling of a model id, so equivalent forms are one unit.
 *
 * `sonnet` and `claude:sonnet` are the same Claude model; counting them as two
 * showed a sweep and a charge the run never makes. Mirrors `canonical_spec` in
 * plan.rs. `claude:` alone is kept — it is the configured default, not a model.
 */
function canonicalUnitId(raw: string): string {
  const id = raw.trim();
  if (!id) return "";
  const { vendor, model } = splitId(id);
  const normalized = model.trim();
  if (vendor === "claude") {
    return normalized || (id.startsWith("claude:") ? "claude:" : "");
  }
  return `${vendor}:${normalized}`;
}

function unitKeys(models: ModelSetting[]): Set<string> {
  const keys = new Set<string>();
  for (const model of models) {
    const canonical = canonicalUnitId(model.id);
    if (!canonical) continue;
    const id = `${canonical}\0${model.use_agents ?? false}`;
    for (const lane of model.lanes) {
      if (!(LANES as readonly string[]).includes(lane)) continue;
      for (let pass = 1; pass <= passesOf(model); pass++) {
        keys.add(`${id}\0${lane}\0${(model.effort ?? "").trim()}\0${pass}`);
      }
    }
  }
  return keys;
}

/** How many (model × lane × pass) sweeps a configuration implies. */
export function unitCount(models: ModelSetting[]): number {
  return unitKeys(models).size;
}

/**
 * How many rounds a run takes.
 *
 * Every vendor but Claude runs one sweep at a time, so its rounds are its unit
 * count; Claude runs `claudeSessions` of them together, so its units divide by
 * that. The run takes as many rounds as its slowest vendor needs. Mirrors
 * `Plan::batches` in plan.rs, and has to: this is the number the window shows
 * before anyone commits to paying for the run.
 */
/**
 * The most Claude sessions the engine will run at once, whatever the plan
 * asks for. Mirrors `MAX_CLAUDE_SESSIONS` in vendor_slots.rs; past it the
 * account's rate limit is reached long before the machine's.
 */
const MAX_CLAUDE_SESSIONS = 8;

function unitsPerVendor(models: ModelSetting[]): Map<string, number> {
  const perVendor = new Map<string, number>();
  for (const key of unitKeys(models)) {
    const vendor = vendorOf(key.split("\0")[0] ?? "");
    perVendor.set(vendor, (perVendor.get(vendor) ?? 0) + 1);
  }
  return perVendor;
}

/**
 * How many Claude sessions a run of this configuration gets.
 *
 * Mirrors `size_claude_sessions_for` in vendor_slots.rs: one per Claude sweep
 * the batch has in flight — the sweeps of one repository times the
 * repositories reviewed together — capped at the ceiling and never below one.
 */
export function claudeSessionsFor(
  models: ModelSetting[],
  repositories = 1,
): number {
  const active = Math.max(Math.trunc(repositories) || 1, 1);
  const claude = unitsPerVendor(models).get("claude") ?? 0;
  return Math.min(Math.max(claude * active, 1), MAX_CLAUDE_SESSIONS);
}

/**
 * How many rounds a run takes per repository.
 *
 * Mirrors `Plan::batches` in plan.rs: every vendor but Claude runs one sweep
 * at a time, and Claude runs as many as the sessions the run is sized to.
 */
export function batchCount(models: ModelSetting[], repositories = 1): number {
  const sessions = claudeSessionsFor(models, repositories);
  const perVendor = unitsPerVendor(models);
  if (perVendor.size === 0) return 0;
  const rounds = [...perVendor].map(([vendor, units]) =>
    vendor === "claude" ? Math.ceil(units / sessions) : units,
  );
  return Math.max(...rounds);
}
