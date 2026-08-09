/**
 * Tests for the frontend's run readiness rules.
 *
 * These are the checks that decide whether the Run button is enabled and
 * whether the settings shown to the user are ones the backend will actually
 * accept.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import {
  type ModelSetting,
  type Settings,
  MAX_PASSES,
  canRun,
  passChoices,
  preset,
  supportsAgents,
} from "./model.ts";

function row(id: string, lanes: string[]): ModelSetting {
  return { id, lanes, effort: "", passes: 1 };
}

test("a run needs both a repository and at least one sweep", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({ ...base, repo: "", models: preset("balanced") }),
    false,
  );
  assert.equal(canRun({ ...base, repo: "C:/x", models: [] }), false);
  assert.equal(
    canRun({ ...base, repo: "C:/x", models: preset("balanced") }),
    true,
  );
});

test("a blank row makes the configuration unrunnable, exactly like the engine", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [row("sonnet", ["correctness"]), row("  ", [])],
    }),
    false,
    "Run must not be enabled with a half-typed row",
  );
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [row("sonnet", ["correctness", "nonsense"])],
    }),
    false,
    "an unknown lane must not be silently ignored",
  );
});

test("agents are available for Claude and Codex but not Kilo Ask", () => {
  assert.equal(supportsAgents("sonnet"), true);
  assert.equal(supportsAgents("codex:"), true);
  assert.equal(supportsAgents("kilo:model"), false);

  const settings = {
    repo: "C:/x",
    scope: "",
    models: [
      {
        id: "kilo:model",
        lanes: ["security"],
        effort: "",
        use_agents: true,
        passes: 1,
      },
    ],
    theme: "system" as const,
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  };
  assert.equal(canRun(settings), false);
});

test("a stored pass count outside the picker presets remains visible", () => {
  // Rust caps `passes` at MAX_PASSES (25) and refuses the run above it, so a
  // stored value within the cap is a valid backend instruction. The selector
  // must offer it rather than silently showing 1 while Run still sends 5.
  assert.deepEqual(passChoices(5), [1, 2, 3, 5]);
  assert.ok(passChoices(5).includes(5));
  // Pre-passes settings (no field) and Rust's max(1) both normalise to one.
  assert.deepEqual(passChoices(undefined), [1, 2, 3]);
  assert.deepEqual(passChoices(0), [1, 2, 3]);
  // The usual values are unchanged and not duplicated.
  assert.deepEqual(passChoices(2), [1, 2, 3]);
});

test("a passes count above the backend cap is not runnable and is clamped in the picker", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [
        { id: "sonnet", lanes: ["correctness"], effort: "", passes: 26 },
      ],
    }),
    false,
    "Run must be disabled for a config the backend would refuse",
  );
  assert.ok(
    !passChoices(40).some((n) => n > MAX_PASSES),
    "the picker must not advertise a pass count above the backend cap",
  );
});
