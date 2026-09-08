import { vendorCliPresent } from "./cli-offer.ts";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  VENDORS,
  runBlockReason,
  type Settings,
  splitId,
  joinId,
  supportsAgents,
  batchCount,
  effortIsValid,
} from "./model.ts";

test("OpenCode local model tags survive selection and storage", () => {
  const id = "opencode:ollama/qwen3:8b";
  const selected = splitId(id);
  assert.deepEqual(selected, { vendor: "opencode", model: "ollama/qwen3:8b" });
  assert.equal(joinId(selected.vendor, selected.model), id);
  assert.equal(supportsAgents(id), false);
  assert.equal(effortIsValid(id, "thinking", {}), true);
  assert.equal(
    batchCount([
      { id, effort: "", lanes: ["security"], passes: 1, use_agents: false },
      {
        id: "sonnet",
        effort: "",
        lanes: ["security"],
        passes: 1,
        use_agents: false,
      },
    ]),
    1,
  );
});

test("retired provider settings remain readable but cannot run or apply", () => {
  for (const id of ["kilo:provider/model", "kimi:model"]) {
    const parsed = splitId(id);
    assert.equal(joinId(parsed.vendor, parsed.model), id);
    assert.equal(vendorCliPresent(id, {}), false);
    assert.match(
      runBlockReason(
        {
          repo: "/repo",
          models: [{ id, lanes: ["security"], effort: "" }],
        } as Settings,
        {},
      )!,
      /support has been removed/,
    );
  }
  assert.deepEqual(VENDORS, ["claude", "codex", "cursor", "opencode"]);
});
