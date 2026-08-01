/**
 * The frontend's own state rules.
 *
 * Kept apart from the DOM so they can be reasoned about — and tested — without
 * a window. Nothing here talks to Tauri.
 */

export const LANES = ["correctness", "security", "contract", "ux"] as const;
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
};

export interface ModelSetting {
  id: string;
  lanes: string[];
  /** Reasoning effort. Empty means the vendor's own default. */
  effort: string;
}

/** The vendors that can be picked, in the order they are offered. */
export const VENDORS = ["claude", "codex", "kilo"] as const;
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
  prove_top: number;
  test_command: string;
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

/** How many (model × lane) sweeps a configuration implies. */
export function unitCount(models: ModelSetting[]): number {
  return models.reduce((total, model) => {
    const valid = new Set(model.lanes.filter((l) => (LANES as readonly string[]).includes(l)));
    return total + (model.id.trim() ? valid.size : 0);
  }, 0);
}

/**
 * The vendor a model spec belongs to. A bare name means Claude, matching the
 * engine's own parsing — these must agree or the UI's batch estimate is wrong.
 */
export function vendorOf(modelId: string): string {
  const [prefix] = modelId.split(":");
  return prefix === "codex" || prefix === "kilo" || prefix === "claude" ? prefix : "claude";
}

/**
 * How many rounds a run takes.
 *
 * The engine never runs two invocations of the same vendor at once, so the
 * number of rounds is the largest number of sweeps any single vendor has to do.
 * Worth showing before a run because it, not the sweep count, is what decides
 * how long someone waits.
 */
export function batchCount(models: ModelSetting[]): number {
  const perVendor = new Map<string, number>();
  for (const model of models) {
    if (!model.id.trim()) continue;
    const lanes = new Set(model.lanes.filter((l) => (LANES as readonly string[]).includes(l)));
    if (lanes.size === 0) continue;
    const vendor = vendorOf(model.id);
    perVendor.set(vendor, (perVendor.get(vendor) ?? 0) + lanes.size);
  }
  return perVendor.size === 0 ? 0 : Math.max(...perVendor.values());
}

/** Whether a configuration could run at all. */
export function canRun(settings: Settings): boolean {
  return settings.repo.trim().length > 0 && unitCount(settings.models) > 0;
}

export type Preset = "cheap" | "balanced" | "deep";

/**
 * The three shipped configurations.
 *
 * Every one covers all four lanes. A preset that quietly left a lane unswept
 * would be the worst possible default, because a preset is exactly what someone
 * picks when they have not thought about lane coverage.
 */
export function preset(name: Preset): ModelSetting[] {
  switch (name) {
    case "cheap":
      // One vendor, every lane. Fewest invocations that still reviews everything.
      return [{ id: "haiku", lanes: [...LANES], effort: "" }];
    case "deep":
      // Every vendor on every lane it can do. Kilo is excluded from nothing here
      // because sweeping is the one thing it does as well as the others.
      return [
        { id: "opus", lanes: [...LANES], effort: "" },
        { id: "codex:", lanes: [...LANES], effort: "" },
        { id: "kilo:", lanes: ["correctness", "security"], effort: "" },
      ];
    case "balanced":
    default:
      return [
        { id: "sonnet", lanes: [...LANES], effort: "" },
        { id: "codex:", lanes: ["correctness", "security"], effort: "" },
      ];
  }
}

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

/**
 * Narrow a grouped model list to those matching a query.
 *
 * Kilo offers 638 models, which is more than a menu can usefully present, so
 * the picker has a filter box. Matching is case-insensitive substring over the
 * whole id — the id already contains the route, the vendor and the model name,
 * so "glm" and "openrouter" and "openrouter/z-ai" all narrow usefully without
 * needing a query syntax.
 *
 * Space-separated terms must ALL match, in any order: "glm openrouter" finds
 * the OpenRouter GLM models whichever way round you think of them.
 *
 * Groups left with nothing are dropped rather than shown empty, so the group
 * headings that remain are a true summary of what is on offer.
 */
export function filterGroups(groups: ModelGroup[], query: string): ModelGroup[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return groups;
  return groups
    .map((group) => ({
      label: group.label,
      models: group.models.filter((id) => {
        const haystack = id.toLowerCase();
        return terms.every((term) => haystack.includes(term));
      }),
    }))
    .filter((group) => group.models.length > 0);
}
