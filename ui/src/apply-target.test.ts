import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { settingsForApply, type Settings } from "./model.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const read = (name: string) => fs.readFileSync(path.join(here, name), "utf8");

test("applying uses the repository that produced the displayed prompt", () => {
  const live: Settings = {
    repo: "C:/repo-b",
    scope: "",
    models: [],
    theme: "system",
    reuse_completed: true,
    provider_concurrency: 3,
    triage_model: "",
    apply_model: "sonnet",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  };
  const sent = settingsForApply(live, "C:/repo-a");
  assert.equal(sent.repo, "C:/repo-a");
  assert.equal(
    live.repo,
    "C:/repo-b",
    "building the request mutated live settings",
  );

  const run = read("run.ts");
  const apply = read("apply.ts");
  const main = read("main.ts");
  assert.ok(
    apply.includes('invoke("apply_fixes"'),
    "the real Apply command disappeared",
  );
  assert.match(run, /currentFixPromptRepo/);
  assert.match(main, /promptRepo:\s*currentFixPromptRepo/);
  assert.match(apply, /settingsForApply\(deps\.settings\(\), repo\)/);
});
