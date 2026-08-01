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
      return [{ id: "haiku", lanes: [...LANES] }];
    case "deep":
      // Every vendor on every lane it can do. Kilo is excluded from nothing here
      // because sweeping is the one thing it does as well as the others.
      return [
        { id: "opus", lanes: [...LANES] },
        { id: "codex:", lanes: [...LANES] },
        { id: "kilo:", lanes: ["correctness", "security"] },
      ];
    case "balanced":
    default:
      return [
        { id: "sonnet", lanes: [...LANES] },
        { id: "codex:", lanes: ["correctness", "security"] },
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
