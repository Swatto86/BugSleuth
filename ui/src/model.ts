/**
 * The frontend's own state rules.
 *
 * Kept apart from the DOM so they can be reasoned about — and tested — without
 * a window. Nothing here talks to Tauri.
 */

import type { Catalogue } from "./view";
import { vendorCliPresent } from "./cli-offer.ts";

/**
 * The most passes a model may request.
 *
 * Must equal `MAX_PASSES` in `crates/bugsleuth-engine/src/plan.rs`, which
 * hard-rejects any run whose passes exceed it. Keeping the same number on both
 * sides stops the UI from enabling a run the backend will refuse.
 */
export const MAX_PASSES = 25;

export const LANES = [
  "correctness",
  "security",
  "contract",
  "ux",
  "gate",
] as const;
export type Lane = (typeof LANES)[number];

/**
 * How each lane is written for a person.
 *
 * A table rather than capitalising the first letter, because that turns "ux"
 * into "Ux". These must match `Lane::title()` in the engine — the same lane
 * named two ways in one product is a small thing that reads as carelessness.
 */
export const LANE_TITLES: Record<Lane, string> = {
  correctness: "Correctness",
  security: "Security",
  contract: "Contract",
  ux: "UX",
  gate: "Gate",
};

export interface ModelSetting {
  id: string;
  lanes: string[];
  /** Reasoning effort. Empty means the vendor's own default. */
  effort: string;
  /** Ask supported providers to delegate independent work in parallel. */
  use_agents?: boolean;
  /**
   * How many times to sweep each lane with this model.
   *
   * Repetition is not redundancy. Three identical sweeps of one fixture each
   * returned five findings and six between them — the same model finds slightly
   * different things each time. Two passes is the cheapest recall available:
   * no second vendor, no second subscription.
   */
  passes: number;
}

/** The vendors that can be picked, in the order they are offered. */
export const VENDORS = ["claude", "codex", "kilo", "kimi", "cursor"] as const;
export type Vendor = (typeof VENDORS)[number];

/**
 * Split a stored spec into the two things the UI lets you choose separately.
 *
 * The stored form stays `vendor:model` because that is what the engine parses;
 * splitting it is a display concern and must not change what is written back.
 * A bare name means Claude, exactly as the engine reads it.
 */
export function splitId(id: string): { vendor: Vendor; model: string } {
  const at = id.indexOf(":");
  if (at === -1) return { vendor: "claude", model: id };
  const prefix = id.slice(0, at);
  return (VENDORS as readonly string[]).includes(prefix)
    ? { vendor: prefix as Vendor, model: id.slice(at + 1) }
    : { vendor: "claude", model: id };
}

/**
 * Put a vendor and model back together.
 *
 * Claude keeps the bare form so an existing settings file and a freshly-picked
 * `sonnet` are the same string, rather than two spellings of one model that
 * would sweep twice and report as two.
 */
export function joinId(vendor: Vendor, model: string): string {
  const trimmed = model.trim();
  return vendor === "claude" ? trimmed : `${vendor}:${trimmed}`;
}

export interface Settings {
  repo: string;
  scope: string;
  models: ModelSetting[];
  theme: "system" | "light" | "dark";
  reuse_completed: boolean;
  /**
   * Model that re-grades every severity once the sweeps are merged, seeing the
   * whole list at once. Empty turns the pass off.
   *
   * Severity is the only thing that orders the report, and each sweep grades
   * its own findings in isolation — measured wrong 6 times in 14.
   */
  triage_model: string;
  /**
   * Model that applies the fixes after a run, as a `vendor:model` spec.
   *
   * Chosen separately from the sweep matrix: finding a defect and fixing it are
   * different jobs, and the cheap model you would spend on the read-only lanes
   * is not necessarily the one you want editing your code. Empty until chosen —
   * the button refuses rather than guessing.
   */
  apply_model: string;
  /**
   * Reasoning effort for that model. Empty is the vendor's own default.
   *
   * Its own field rather than part of the spec: effort is not part of a model
   * id, and packing it in would send `opus:high` to a CLI as a model name.
   */
  apply_effort: string;
  /**
   * Push what an apply committed to the branch's existing upstream.
   *
   * Off by default. Every other thing an apply does is undone with `git reset`;
   * this is the one that cannot be, so it is opted into rather than assumed,
   * and Rust refuses it outright when the branch has no upstream or a commit
   * still credits a tool for the work.
   */
  push_after_apply: boolean;
  /**
   * After a successful push, tag the published commits so the repository's own
   * CI cuts a release from them.
   *
   * Off by default, and only ever acted on when the push succeeded — a tag
   * whose commits are not on the remote starts a build of a ref the runner
   * cannot fetch. Rust reads the next version from the branch's own most recent
   * `v*` tag and refuses rather than inventing one for a repository that has no
   * such tag or names its releases some other way.
   */
  tag_release_after_push: boolean;
}

export function settingsForApply(settings: Settings, repo: string): Settings {
  return { ...settings, repo };
}

/**
 * Lanes no model covers.
 *
 * This is the number the UI exists to keep at zero. A lane with nobody assigned
 * still produces a report — it just says "not swept" — and a reader skimming
 * for findings can easily miss that a whole mandate was never applied. Catching
 * it here, before the run, is the only free chance to fix it.
 */
export function uncoveredLanes(models: ModelSetting[]): Lane[] {
  return LANES.filter((lane) => !models.some((m) => m.lanes.includes(lane)));
}

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
        keys.add(`${id} ${lane} ${(model.effort ?? "").trim()} ${pass}`);
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
 * The vendor a model spec belongs to. A bare name means Claude, matching the
 * engine's own parsing — these must agree or the UI's batch estimate is wrong.
 */
export function vendorOf(modelId: string): string {
  return splitId(modelId).vendor;
}

/** Whether BugSleuth's read-only invocation of this provider can delegate. */
export const CANNOT_DELEGATE = ["kilo", "kimi", "cursor"] as const;

/**
 * Whether this vendor can be asked to split a lane across subagents.
 *
 * Must match `sweep::agents::support` in the engine, which is the authority —
 * `bridge.test.ts` compares the two, because both sides are strings and no
 * compiler spans them. It said `!== "kilo"` when Kimi was added, so the box was
 * offered for a vendor that refuses: every lane came back NOT SWEPT and a whole
 * run produced nothing.
 */
export function supportsAgents(modelId: string): boolean {
  return !(CANNOT_DELEGATE as readonly string[]).includes(vendorOf(modelId));
}

/** Whether this Claude selection can use the CLI's Ultracode workflow mode. */
export function usesUltracode(modelId: string): boolean {
  const { vendor, model } = splitId(modelId);
  if (vendor !== "claude") return false;
  const id = model.trim().toLowerCase();
  return (
    ["", "fable", "opus", "sonnet"].includes(id) ||
    [
      "claude-fable-5",
      "claude-opus-5",
      "claude-opus-4-8",
      "claude-opus-4-7",
      "claude-sonnet-5",
    ].some((family) => id.includes(family))
  );
}

/**
 * How many rounds a run takes.
 *
 * The engine runs one sweep per vendor at once, so the run's rounds are the most
 * any single vendor needs. Mirrors `Plan::batches` in plan.rs.
 */
export function batchCount(models: ModelSetting[]): number {
  const perVendor = new Map<string, number>();
  for (const key of unitKeys(models)) {
    const vendor = vendorOf(key.split(" ")[0] ?? "");
    perVendor.set(vendor, (perVendor.get(vendor) ?? 0) + 1);
  }
  return perVendor.size === 0 ? 0 : Math.max(...perVendor.values());
}

/**
 * Whether this model accepts this effort, as the backend judges it.
 *
 * The window kept a stored effort the catalogue no longer lists and disabled
 * the selector whenever the model had no levels — so a persisted `high` on
 * Haiku displayed `high`, locked the only control that could clear it, and left
 * Run and Apply enabled for a configuration Rust rejects in `plan::check_effort`
 * and the providers' own `validate_effort`.
 *
 * Deliberately permissive where the backend is: Kilo variants and the efforts of
 * an unlisted custom Claude model cannot be enumerated, so neither is refused
 * here. When the catalogue itself failed to load there is nothing to judge
 * against, and backend validation stays authoritative rather than every
 * configured action being disabled for the session.
 */
export function effortIsValid(
  id: string,
  rawEffort: string,
  catalogue: Catalogue,
): boolean {
  const effort = rawEffort.trim();
  if (effort === "") return true;
  const { vendor, model } = splitId(id);
  if (vendor === "kilo") return true;
  const menu = catalogue[vendor];
  if (!menu) return true;
  if (menu.efforts.length > 0) return menu.efforts.includes(effort);
  const accepted = menu.efforts_by_model[model];
  if (accepted !== undefined) return accepted.includes(effort);
  return vendor === "claude";
}

/** Whether a configuration could run at all. */
export function canRun(settings: Settings, catalogue: Catalogue): boolean {
  if (settings.repo.trim().length === 0) return false;
  // Mirrors plan::plan: a blank model id or an unknown lane makes the whole
  // configuration unrunnable, not a row to be ignored. The engine refuses such
  // a configuration outright, so the button must not promise a run the backend
  // will reject with "a configured model has an empty id".
  if (
    settings.models.length === 0 ||
    !settings.models.every(
      (model) =>
        model.id.trim() !== "" &&
        vendorCliPresent(model.id, catalogue) &&
        effortIsValid(model.id, model.effort, catalogue) &&
        (!model.use_agents || supportsAgents(model.id)) &&
        (!model.use_agents ||
          !usesUltracode(model.id) ||
          model.effort.trim() === "") &&
        model.lanes.every((lane) =>
          (LANES as readonly string[]).includes(lane),
        ) &&
        (model.passes ?? 1) <= MAX_PASSES,
    )
  ) {
    return false;
  }
  return unitCount(settings.models) > 0;
}

/**
 * What the status line says once an apply is over.
 *
 * A run that changed nothing is not a run that applied fixes. This said "Fixes
 * applied — 0 files changed. Review the diff." over a repository nobody had
 * touched, and it read as the apply having silently failed. It had not: the
 * model verified all sixteen findings, reported every one as already fixed or
 * not a defect, and correctly refused to edit code into agreement with a stale
 * report. Saying "applied" there is the control claiming an outcome it did not
 * produce — and pointing at a diff that does not exist.
 */
export function applyStatus(ok: boolean, changed: number): string {
  if (!ok) return "Applying the fixes failed";
  if (changed === 0) {
    return "Nothing was changed — read what the model said below";
  }
  return `Fixes applied — ${changed} file${changed === 1 ? "" : "s"} changed. Review the diff.`;
}

// The shipped presets and the whole-matrix equality check live in ./presets to
// keep this file under the line cap, and are re-exported so callers still import
// them from "./model".
export { preset, isShippedConfiguration, type Preset } from "./presets.ts";

/** Toggle one lane for one model, returning a new list. */
export function toggleLane(
  models: ModelSetting[],
  index: number,
  lane: Lane,
  on: boolean,
): ModelSetting[] {
  return models.map((model, i) => {
    if (i !== index) return model;
    const lanes = new Set(model.lanes);
    if (on) lanes.add(lane);
    else lanes.delete(lane);
    return { ...model, lanes: [...lanes] };
  });
}

/** A named set of models offered together. */
export interface ModelGroup {
  label: string;
  models: string[];
}
