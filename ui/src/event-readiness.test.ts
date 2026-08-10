/**
 * A long-running action must not be offered before the event that ends it is
 * being listened for.
 *
 * `run-finished` and `apply-finished` are the only things that clear `running`
 * and `applying`. If either subscription rejects — Tauri event registration
 * failing while command IPC still works — the action still starts, the backend
 * still completes it, and no callback ever arrives: Run, Apply, Clear and Update
 * stay blocked until the app is restarted. The Apply failure was reported only
 * through the shared status, which boot then overwrote with "Ready".
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

test("long-running actions wait for their completion listeners", () => {
  const run = read("ui", "src", "run.ts");
  const main = read("ui", "src", "main.ts");
  const apply = read("ui", "src", "apply.ts");
  const html = read("ui", "index.html");

  // Readiness is set after the subscription that grants it, not before.
  const runListener = run.indexOf('"run-finished"');
  const runReady = run.indexOf("completionEventsReady = true", runListener);
  assert.ok(runListener >= 0, "the run-finished listener is gone");
  assert.ok(
    runReady > runListener,
    "run events are marked ready before they are subscribed",
  );
  assert.ok(
    main.includes("!runEventsReady() || busy"),
    "the Run gate does not require its completion listener",
  );

  const applyListener = apply.indexOf('"apply-finished"');
  const applyReady = apply.indexOf(
    "completionEventsReady = true",
    applyListener,
  );
  assert.ok(applyListener >= 0, "the apply-finished listener is gone");
  assert.ok(
    applyReady > applyListener,
    "apply events are marked ready before they are subscribed",
  );
  // Whitespace-normalized: the formatter wraps this expression across lines.
  assert.ok(
    apply.replace(/\s+/g, " ").includes("!completionEventsReady || applying"),
    "the Apply gate does not require its completion listener",
  );

  // A persistent alert, because the shared status is overwritten by boot.
  assert.match(html, /id="apply-listener-error"[^>]*role="alert"/);
  // Disabled in shipped markup too, so the pre-JavaScript window is not a way in.
  assert.match(html, /id="run"[^>]*disabled/);
});
