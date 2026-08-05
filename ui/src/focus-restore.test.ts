/**
 * Removing the focused final model row must not drop keyboard and screen-reader
 * focus to `<body>`. The exact focus movement is exercised end-to-end in
 * e2e/specs/review.spec.ts; this reads main.ts's own syntax so a regression to
 * a bare `restored?.focus()` — which leaves focus nowhere when the row is gone —
 * is caught without a live app.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { frontendFiles, walk } from "./ast.test.ts";

/** The source text of a top-level function declaration, by name. */
function functionText(source: ts.SourceFile, name: string): string | undefined {
  let text: string | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) {
      text = node.getText();
    }
  });
  return text;
}

test("render redirects focus when the focused row was just removed", () => {
  const main = frontendFiles().find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");

  const render = functionText(main, "render");
  assert.ok(render, "render() is gone from main.ts");

  // When the focused control was the removed row's own Remove button, its focus
  // key no longer exists, so the rebuild must continue at the new final row or
  // at Add model rather than leaving focus on <body>.
  assert.match(
    render,
    /remove-/,
    "render no longer keys off the removed row's remove- focus key",
  );
  assert.match(
    render,
    /addModel/,
    "render has no Add model focus fallback for an emptied matrix",
  );
});
