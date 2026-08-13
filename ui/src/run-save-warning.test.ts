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
import ts from "typescript";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("a partly-failed handoff save is surfaced, not swallowed", () => {
  // The payload moved to outcome.rs when start_run hit the function-length
  // limit; the check follows the code rather than staying pointed at the file
  // that used to hold it, where it would pass on an absence.
  const runRs = read("src-tauri", "src", "outcome.rs");
  assert.ok(
    !/write_all\([\s\S]*?\)\s*\.ok\(\)/.test(runRs),
    "the run command still drops write_all's result with .ok()",
  );
  assert.ok(
    runRs.includes('"saveError"'),
    "the run payload no longer carries saveError, so a failed save is invisible",
  );
  assert.ok(
    /"ok": true,\s*"complete": complete,/.test(runRs),
    "the run payload no longer says whether every lane was swept",
  );

  // Scoped to the run-finished callback. `runTs.includes("saveError")` also
  // matched the payload's own type declaration, so deleting the handling branch
  // left this green while a half-saved run reported a plain Finished.
  const runTs = read("ui", "src", "run.ts");
  const source = ts.createSourceFile(
    "run.ts",
    runTs,
    ts.ScriptTarget.ESNext,
    true,
  );
  const walk = (node: ts.Node, visit: (candidate: ts.Node) => void): void => {
    visit(node);
    node.forEachChild((child) => walk(child, visit));
  };
  let finished: ts.Expression | undefined;
  walk(source, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "listen" &&
      node.arguments[0] !== undefined &&
      ts.isStringLiteral(node.arguments[0]) &&
      node.arguments[0].text === "run-finished"
    ) {
      finished = node.arguments[1];
    }
  });
  assert.ok(finished, "the run-finished listener is gone");
  let consumesSaveError = false;
  let consumesComplete = false;
  walk(finished, (node) => {
    if (
      ts.isPropertyAccessExpression(node) &&
      node.name.text === "saveError" &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "payload" &&
      ts.isIdentifier(node.expression.expression) &&
      node.expression.expression.text === "event"
    ) {
      consumesSaveError = true;
    }
    if (
      ts.isPropertyAccessExpression(node) &&
      node.name.text === "complete" &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "payload" &&
      ts.isIdentifier(node.expression.expression) &&
      node.expression.expression.text === "event"
    ) {
      consumesComplete = true;
    }
  });
  assert.ok(
    consumesSaveError,
    "the run-finished handler no longer consumes event.payload.saveError",
  );
  assert.ok(
    consumesComplete,
    "the run-finished handler no longer consumes event.payload.complete",
  );
});
