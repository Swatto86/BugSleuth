/**
 * Starting a run must not strand keyboard focus on <body>.
 *
 * renderPlanSummary disables the Run button the moment a run starts, and in
 * WebView2 disabling the focused element resets focus to <body> — leaving a
 * keyboard or screen-reader user unable to reach the live Stop control. startRun
 * moves focus to Stop; this guards that it still does.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { frontendFiles } from "./ast.test.ts";

test("starting a run moves focus off the disabled Run button to Stop", () => {
  const run = frontendFiles().find((f) => f.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  assert.match(
    run.getText(),
    /deps\.stop\.focus\(\)/,
    "startRun never moves focus to Stop, so activating Run drops focus to <body>",
  );
});
