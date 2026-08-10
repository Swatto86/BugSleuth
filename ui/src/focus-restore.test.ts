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

test("a failed catalogue load is told to the user rather than swallowed", () => {
  const main = frontendFiles().find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");
  const load = functionText(main, "loadCatalogue");
  assert.ok(load, "loadCatalogue is gone from main.ts");
  assert.match(load, /catch/, "loadCatalogue has no catch");
  assert.match(
    load,
    /return/,
    "loadCatalogue swallows the failure instead of handing it back",
  );
  const boot = functionText(main, "boot");
  assert.ok(boot, "boot is gone from main.ts");
  assert.match(
    boot,
    /catalogueError/,
    "boot never reads the catalogue failure, so the preflight 'Ready' overwrites it",
  );
});

/// A completed apply must not take focus off the control the user is in.
///
/// The Apply model and effort controls stay enabled for the hours an apply can
/// run. When `apply-finished` arrives its handler calls `draw()`, which replaces
/// both — and in WebView2 replacing the element containing focus drops focus to
/// `<body>`. Unlike the matrix renderer, this redraw captured nothing, so a
/// keyboard user was returned to the top of the application.
test("apply redraw restores a focused picker after replacing it", () => {
  const apply = frontendFiles().find(
    (source) => source.fileName === "apply.ts",
  );
  assert.ok(apply, "apply.ts is no longer a shipped frontend module");

  let draw: ts.ArrowFunction | undefined;
  walk(apply, (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === "draw" &&
      node.initializer &&
      ts.isArrowFunction(node.initializer)
    ) {
      draw = node.initializer;
    }
  });
  assert.ok(draw, "bindApply no longer has a draw function");

  const body = draw.getText(apply);
  const modelReplace = body.indexOf("ui.model.replaceChildren");
  const effortReplace = body.indexOf("drawEffort();");
  const restore = body.lastIndexOf(".focus()");
  assert.ok(
    body.includes("document.activeElement"),
    "draw does not look at where focus is before replacing controls",
  );
  assert.ok(
    body.includes('dataset["focusKey"]'),
    "draw does not capture which control held focus",
  );
  assert.ok(
    modelReplace >= 0 && effortReplace >= 0,
    "draw no longer replaces both pickers; this check needs rewriting",
  );
  assert.ok(
    restore > modelReplace && restore > effortReplace,
    "focus is not restored after both picker replacements",
  );
});
