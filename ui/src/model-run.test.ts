/**
 * Tests for the frontend's run readiness rules.
 *
 * These are the checks that decide whether the Run button is enabled and
 * whether the settings shown to the user are ones the backend will actually
 * accept.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import { test } from "node:test";

import {
  type ModelSetting,
  type Settings,
  MAX_PASSES,
  canRun,
  effortIsValid,
  passChoices,
  preset,
  runBlockReason,
  supportsAgents,
  usesUltracode,
} from "./model.ts";
import { offeredVendors } from "./cli-offer.ts";
import type { Catalogue } from "./view.ts";

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
    canRun({ ...base, repo: "", models: preset("balanced") }, {}),
    false,
  );
  assert.match(
    runBlockReason({ ...base, repo: "", models: preset("balanced") }, {}) ?? "",
    /repository/i,
  );
  assert.equal(canRun({ ...base, repo: "C:/x", models: [] }, {}), false);
  assert.equal(
    canRun({ ...base, repo: "C:/x", models: preset("balanced") }, {}),
    true,
  );
  assert.equal(
    runBlockReason({ ...base, repo: "C:/x", models: preset("balanced") }, {}),
    null,
  );
});

test("codex can be scheduled for repository review", () => {
  assert.equal(
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [row("codex:", ["correctness"])],
      },
      {},
    ),
    true,
  );
  assert.equal(
    runBlockReason(
      {
        ...base,
        repo: "C:/x",
        models: [row("codex:", ["correctness"])],
      },
      {},
    ),
    null,
  );
  for (const name of ["balanced", "deep"] as const) {
    assert.ok(
      preset(name).every((model) => !model.id.startsWith("codex:")),
      `${name} still ships a Codex review row`,
    );
  }
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
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [row("sonnet", ["correctness"]), row("  ", [])],
      },
      {},
    ),
    false,
    "Run must not be enabled with a half-typed row",
  );
  assert.match(
    runBlockReason(
      {
        ...base,
        repo: "C:/x",
        models: [row("sonnet", ["correctness"]), row("  ", [])],
      },
      {},
    ) ?? "",
    /empty row|model id/,
  );
  assert.equal(
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [row("sonnet", ["correctness", "nonsense"])],
      },
      {},
    ),
    false,
    "an unknown lane must not be silently ignored",
  );
});

test("agents are available for Claude and Codex but not Kilo Ask", () => {
  assert.equal(supportsAgents("sonnet"), true);
  assert.equal(supportsAgents("codex:"), true);
  assert.equal(supportsAgents("kilo:model"), false);
  assert.equal(usesUltracode("fable"), true);
  assert.equal(usesUltracode("sonnet"), true);
  assert.equal(usesUltracode("haiku"), false);
  assert.equal(usesUltracode("codex:gpt-5.6-codex"), false);

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
  assert.equal(canRun(settings, {}), false);

  settings.models[0] = {
    id: "fable",
    lanes: ["security"],
    effort: "high",
    use_agents: true,
    passes: 1,
  };
  assert.equal(
    canRun(settings, {}),
    false,
    "Ultracode replaces explicit effort",
  );
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
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [
          { id: "sonnet", lanes: ["correctness"], effort: "", passes: 26 },
        ],
      },
      {},
    ),
    false,
    "Run must be disabled for a config the backend would refuse",
  );
  assert.ok(
    !passChoices(40).some((n) => n > MAX_PASSES),
    "the picker must not advertise a pass count above the backend cap",
  );
});

/// A stored effort the backend will reject must not leave the action enabled.
///
/// The frontend preserves an unlisted stored effort and then disables the
/// selector whenever the model has no levels — so Haiku with a persisted `high`
/// showed `high`, locked the only control that could clear it, and left Run
/// enabled for a configuration `plan::check_effort` rejects outright. The Apply
/// panel had the same shape through `apply_model`/`apply_effort`.
test("an effort the model does not accept blocks the action", () => {
  const catalogue: Catalogue = {
    claude: {
      vendor: "claude",
      error: "",
      installed: true,
      groups: [],
      efforts: [],
      efforts_by_model: { haiku: [], sonnet: ["high", "low"] },
    },
  };
  const withModel = (id: string, effort: string): Settings => ({
    ...base,
    repo: "C:/x",
    models: [{ id, lanes: ["correctness"], effort, passes: 1 }],
  });

  assert.equal(
    canRun(withModel("haiku", "high"), catalogue),
    false,
    "Run was enabled for an effort Rust rejects before the sweep starts",
  );
  assert.equal(canRun(withModel("haiku", ""), catalogue), true);
  assert.equal(canRun(withModel("sonnet", "high"), catalogue), true);
  // Deliberately open: the backend cannot enumerate Kilo variants or the
  // efforts of a custom Claude model, so neither may be refused here.
  assert.equal(
    canRun(withModel("kilo:some/model", "anything"), catalogue),
    true,
  );
  assert.equal(
    canRun(withModel("custom-claude-model", "high"), catalogue),
    true,
  );

  // The exported helper the Apply gate uses answers the same way.
  assert.equal(effortIsValid("haiku", "high", catalogue), false);
  assert.equal(effortIsValid("haiku", "", catalogue), true);
  assert.equal(effortIsValid("sonnet", "low", catalogue), true);
  assert.equal(effortIsValid("kilo:some/model", "anything", catalogue), true);
  // With no catalogue at all, backend validation stays authoritative rather
  // than every configured action being disabled for the session.
  assert.equal(effortIsValid("haiku", "high", {}), true);
});

test("menus only offer vendors whose CLI is installed", () => {
  const catalogue: Catalogue = {
    claude: {
      vendor: "claude",
      installed: true,
      error: null,
      groups: [],
      efforts: [],
      efforts_by_model: {},
    },
    codex: {
      vendor: "codex",
      installed: false,
      error: "Codex CLI not found",
      groups: [],
      efforts: [],
      efforts_by_model: {},
    },
    kilo: {
      vendor: "kilo",
      installed: true,
      error: null,
      groups: [],
      efforts: [],
      efforts_by_model: {},
    },
  };
  assert.deepEqual(offeredVendors(catalogue), ["claude", "kilo"]);
  // A stale saved vendor stays visible so the user can switch away from it.
  assert.deepEqual(offeredVendors(catalogue, "codex"), [
    "claude",
    "kilo",
    "codex",
  ]);
  assert.ok(
    offeredVendors(catalogue, "codex").includes("codex"),
    "a stale saved Codex row stays visible so it can be changed",
  );
  const withCodex = {
    ...catalogue,
    codex: { ...catalogue.codex, installed: true },
  };
  assert.ok(
    offeredVendors(withCodex).includes("codex"),
    "an installed Codex CLI must appear in the sweep provider list",
  );
  assert.ok(
    offeredVendors({}).includes("codex"),
    "Codex stays visible before the catalogue loads rather than vanishing at boot",
  );
  // Before the catalogue loads, every known vendor stays offered.
  assert.ok(offeredVendors({}).includes("cursor"));
  const view = fs.readFileSync(new URL("./view.ts", import.meta.url), "utf8");
  assert.ok(
    view.includes("offeredVendors("),
    "the sweep matrix must offer providers through offeredVendors",
  );
  assert.ok(
    !view.includes("offeredSweepVendors"),
    "a second filter would hide an installed provider the status pills still show",
  );
});
