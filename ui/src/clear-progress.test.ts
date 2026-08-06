/**
 * Clearing saved sweeps must show the same in-progress feedback as every other
 * backend operation: a running status and a disabled trigger. Without this, a slow
 * delete looks like nothing happened and the button can be pressed again, leading
 * to a spurious "could not clear" error while the first delete succeeds.
 *
 * A real double-click during a slow delete needs precise timing, so this reads
 * actions.ts's own syntax instead: the clear handler must set a running status and
 * disable ui.clearSaved before the invoke, and re-enable it on both outcomes.
 *
 * Deliberately self-contained rather than importing the AST helpers: importing
 * another *.test.ts module re-runs its tests, which the inventory then records
 * as duplicates.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));

function actionsSource(): { source: ts.SourceFile; text: string } {
  const file = path.join(here, "actions.ts");
  const text = fs.readFileSync(file, "utf8");
  return {
    source: ts.createSourceFile(
      "actions.ts",
      text,
      ts.ScriptTarget.ESNext,
      true,
    ),
    text,
  };
}

/** The source text of the arrow-function listener attached to clearSaved. */
function clearHandlerText(source: ts.SourceFile): string | undefined {
  let found: string | undefined;
  const walk = (node: ts.Node): void => {
    if (
      ts.isPropertyAccessExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "ui" &&
      ts.isIdentifier(node.name) &&
      node.name.text === "clearSaved" &&
      node.parent &&
      ts.isPropertyAccessExpression(node.parent) &&
      ts.isIdentifier(node.parent.name) &&
      node.parent.name.text === "addEventListener" &&
      node.parent.parent &&
      ts.isCallExpression(node.parent.parent) &&
      node.parent.parent.arguments.length >= 2 &&
      ts.isArrowFunction(node.parent.parent.arguments[1])
    ) {
      found = node.parent.parent.arguments[1].getText();
    }
    node.forEachChild(walk);
  };
  walk(source);
  return found;
}

test("the clear handler shows a running status while the delete is in flight", () => {
  const handler = clearHandlerText(actionsSource().source);
  assert.ok(handler, "ui.clearSaved listener is gone from actions.ts");
  assert.match(
    handler,
    /setStatus\([^)]*"running"/,
    "clear_saved does not set a running status, so the user sees no spinner",
  );
});

test("the clear handler disables its button and re-enables it on both outcomes", () => {
  const handler = clearHandlerText(actionsSource().source);
  assert.ok(handler, "ui.clearSaved listener is gone from actions.ts");
  assert.match(
    handler,
    /ui\.clearSaved\.disabled = true/,
    "clear_saved does not disable its button, so it can be triggered again",
  );
  const reEnableMatches = handler.match(/ui\.clearSaved\.disabled = false/g) ?? [];
  assert.ok(
    reEnableMatches.length >= 2,
    `the button is re-enabled on ${reEnableMatches.length} path(s); it must be ` +
      "re-enabled on both the success and the failure of clear_saved",
  );
});
