/**
 * The shipped configurations and the check that a matrix is exactly one of them.
 *
 * Split from model.ts at the hard line cap; re-exported from there so callers
 * still import `preset` / `isShippedConfiguration` from "./model".
 */

import { LANES, type ModelSetting } from "./model.ts";

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
      return [{ id: "haiku", lanes: [...LANES], effort: "", passes: 1 }];
    case "deep":
      // Every vendor on every lane it can do. Kilo is excluded from nothing here
      // because sweeping is the one thing it does as well as the others.
      return [
        { id: "opus", lanes: [...LANES], effort: "", passes: 1 },
        { id: "codex:", lanes: [...LANES], effort: "", passes: 1 },
        {
          id: "kilo:",
          lanes: ["correctness", "security"],
          effort: "",
          passes: 1,
        },
      ];
    case "balanced":
    default:
      return [
        { id: "sonnet", lanes: [...LANES], effort: "", passes: 1 },
        {
          id: "codex:",
          lanes: ["correctness", "security"],
          effort: "",
          passes: 1,
        },
      ];
  }
}

/**
 * Whether the whole matrix equals one shipped preset, exactly.
 *
 * The confirmation before a preset replace is skipped only when nothing is at
 * stake. Checking each row against "is a shipped row" let an arbitrary mixture,
 * subset or duplicate of shipped rows through — a user-built matrix destroyed
 * without a prompt. Equality is of the complete ordered configuration: row
 * count, and each row's model, effort, passes and lanes.
 */
export function isShippedConfiguration(models: ModelSetting[]): boolean {
  return (["cheap", "balanced", "deep"] as const).some((name) => {
    const shipped = preset(name);
    return (
      shipped.length === models.length &&
      shipped.every((expected, index) => {
        const actual = models[index];
        return (
          actual !== undefined &&
          actual.id === expected.id &&
          actual.effort === expected.effort &&
          (actual.passes ?? 1) === (expected.passes ?? 1) &&
          actual.lanes.length === expected.lanes.length &&
          expected.lanes.every((lane) => actual.lanes.includes(lane))
        );
      })
    );
  });
}
