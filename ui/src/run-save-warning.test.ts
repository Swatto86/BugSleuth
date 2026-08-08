/**
 * A handoff save that half-failed must reach the user, not be swallowed.
 *
 * The Tauri run command used to drop `write_all`'s result with `.ok()`, so a
 * review whose fix prompts failed to save still reported Finished. This guards
 * the two ends of the wiring that fixed it: Rust emits a `saveError`, and the
 * frontend consumes it. Neither has a compiler spanning the boundary between
 * them, so this is where they are checked to agree.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("a partly-failed handoff save is surfaced, not swallowed", () => {
  const runRs = read("src-tauri", "src", "commands", "run.rs");
  assert.ok(
    !/write_all\([\s\S]*?\)\s*\.ok\(\)/.test(runRs),
    "the run command still drops write_all's result with .ok()",
  );
  assert.ok(
    runRs.includes('"saveError"'),
    "the run payload no longer carries saveError, so a failed save is invisible",
  );

  const runTs = read("ui", "src", "run.ts");
  assert.ok(
    runTs.includes("saveError"),
    "run.ts no longer consumes saveError, so the status cannot reflect it",
  );
});
