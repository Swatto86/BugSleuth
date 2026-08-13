/** Provider installation gates shared by the model picker and Run action. */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { vendorCliPresent } from "./cli-offer.ts";
import { canRun, type Settings } from "./model.ts";
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

test("Run refuses a model whose CLI is not installed", () => {
  const catalogue: Catalogue = {
    claude: {
      vendor: "claude",
      installed: false,
      error: "not found",
      groups: [{ label: "Claude", models: ["haiku"] }],
      efforts: [],
      efforts_by_model: {},
    },
  };
  assert.equal(vendorCliPresent("haiku", catalogue), false);
  assert.equal(
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [
          { id: "haiku", lanes: ["correctness"], effort: "", passes: 1 },
        ],
      },
      catalogue,
    ),
    false,
    "Run stayed enabled for a provider whose CLI is not on the machine",
  );
  // Empty catalogue (still loading) must not block.
  assert.equal(vendorCliPresent("haiku", {}), true);

  const codexOnly: Catalogue = {
    claude: { ...catalogue.claude!, installed: false },
    codex: {
      vendor: "codex",
      installed: true,
      error: "",
      groups: [{ label: "Codex", models: ["gpt"] }],
      efforts: [],
      efforts_by_model: {},
    },
  };
  const spacedCodex = " codex:gpt ";
  assert.equal(vendorCliPresent(spacedCodex, codexOnly), true);
  assert.equal(
    canRun(
      {
        ...base,
        repo: "C:/x",
        models: [
          { id: spacedCodex, lanes: ["correctness"], effort: "", passes: 1 },
        ],
      },
      codexOnly,
    ),
    true,
  );
  assert.equal(
    vendorCliPresent(spacedCodex, {
      ...codexOnly,
      claude: { ...codexOnly.claude!, installed: true },
      codex: { ...codexOnly.codex!, installed: false },
    }),
    false,
    "a spaced Codex ID was checked against Claude's install state",
  );
});
