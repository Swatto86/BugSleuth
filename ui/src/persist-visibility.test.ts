/**
 * A settings-save failure must be visible even during a run.
 *
 * The old handler suppressed the error whenever a run was in progress — exactly
 * when a save is most likely to fail — so a whole run's configuration could be
 * lost silently. The failure now goes to its own alert region, ungated. This
 * guards that the gate is gone and the region exists.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("a save failure is shown in its own region, never gated on a run", () => {
  const persist = read("ui", "src", "persist.ts");
  assert.ok(
    persist.includes("deps.setError("),
    "the save handler no longer reports through a dedicated error region",
  );
  assert.ok(
    !/quiet/.test(persist),
    "the save error is still suppressed while a run is in progress",
  );

  const html = read("ui", "index.html");
  assert.match(
    html,
    /id="settings-error"[^>]*role="alert"/,
    "the settings-error alert region is missing from the footer",
  );
});
