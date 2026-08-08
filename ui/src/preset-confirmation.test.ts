/**
 * A preset replace skips its confirmation only when nothing is lost.
 *
 * The guard used to test each row against "is a shipped row", so an arbitrary
 * mix, subset or duplicate of shipped rows passed and a user-built matrix was
 * destroyed without a prompt. The handler must key on the whole configuration.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("the preset handler guards on the whole configuration, not per row", () => {
  const actions = read("ui", "src", "actions.ts");
  assert.ok(
    actions.includes("deps.isShippedConfiguration(deps.settings().models)"),
    "the preset handler no longer guards on the whole configuration",
  );
  assert.ok(
    !/models\.every\(deps\.isShipped\)/.test(actions),
    "the per-row guard that let a mixed matrix through is still present",
  );
});
