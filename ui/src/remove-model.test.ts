/**
 * Removing a configured model row destroys a provider, model id, effort, pass
 * count and five lane assignments at once, with no undo. Like preset
 * replacement and clearing saved sweeps, it must pass through a confirmation
 * rather than mutating settings on the spot.
 *
 * The behaviour itself is exercised end-to-end in e2e/specs/review.spec.ts; this
 * reads the frontend's own syntax so a regression is caught without a live app.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { callsTo, frontendFiles, walk } from "./ast.test.ts";

/** The source text of a named object-literal member, wherever it appears. */
function memberText(source: ts.SourceFile, name: string): string | undefined {
  let found: string | undefined;
  walk(source, (node) => {
    if (
      (ts.isPropertyAssignment(node) ||
        ts.isMethodDeclaration(node) ||
        ts.isShorthandPropertyAssignment(node)) &&
      ts.isIdentifier(node.name) &&
      node.name.text === name
    ) {
      found ??= node.getText();
    }
  });
  return found;
}

test("removing a configured model row is guarded by a confirmation", () => {
  const files = frontendFiles();
  const matrix = files.find((file) => file.fileName === "matrix.ts");
  assert.ok(matrix, "matrix.ts is no longer a shipped frontend module");

  // Assert the handler was found first, so a rename cannot make this pass
  // vacuously — an absent handler satisfies every assertion below it.
  const onRemove = memberText(matrix, "onRemove");
  assert.ok(onRemove, "matrix.ts defines no onRemove handler");
  assert.match(
    onRemove,
    /confirmDialog/,
    `the model-row remove handler deletes without confirmation: ${onRemove}`,
  );

  // And those handlers really are the ones the table is built with. A guarded
  // handler in a module nothing wires up would pass the check above while every
  // Remove button in the window deleted on the spot.
  const main = files.find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");
  assert.ok(
    callsTo(main, "matrixHandlers").length >= 1,
    "main.ts no longer builds the row handlers",
  );
  assert.ok(
    callsTo(main, "matrixRows").length >= 1,
    "matrixRows is no longer called from main.ts",
  );
});
