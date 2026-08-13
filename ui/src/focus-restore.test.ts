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

/** A function declaration, by name. */
function functionDeclaration(
  source: ts.SourceFile,
  name: string,
): ts.FunctionDeclaration | undefined {
  let found: ts.FunctionDeclaration | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) {
      found = node;
    }
  });
  return found;
}

const functionText = (source: ts.SourceFile, name: string) =>
  functionDeclaration(source, name)?.getText();

test("render redirects focus when the focused row was just removed", () => {
  // The behaviour moved to plan-view.ts when main.ts was split at the line cap;
  // the check follows it rather than staying pointed at a wrapper that no
  // longer contains it.
  const view = frontendFiles().find((file) => file.fileName === "plan-view.ts");
  assert.ok(view, "plan-view.ts is no longer a shipped frontend module");

  const render = functionText(view, "withRestoredFocus");
  assert.ok(render, "withRestoredFocus() is gone from plan-view.ts");

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
  const load = functionDeclaration(main, "loadCatalogue");
  assert.ok(load, "loadCatalogue is gone from main.ts");
  let failure: ts.CatchClause | undefined;
  walk(load, (node) => {
    if (ts.isCatchClause(node)) failure = node;
  });
  assert.ok(failure, "loadCatalogue has no catch");
  const caught = failure.variableDeclaration?.name;
  assert.ok(
    caught && ts.isIdentifier(caught),
    "the catalogue error is not named",
  );
  let returned: ts.ReturnStatement | undefined;
  walk(failure.block, (node) => {
    if (ts.isReturnStatement(node)) returned = node;
  });
  assert.ok(returned?.expression, "the catalogue failure is not returned");
  const diagnostic = returned.expression;
  assert.ok(
    ts.isTemplateExpression(diagnostic) &&
      diagnostic.head.text.includes("Could not load the model lists") &&
      diagnostic.templateSpans.some(
        ({ expression }) =>
          ts.isCallExpression(expression) &&
          ts.isIdentifier(expression.expression) &&
          expression.expression.text === "String" &&
          expression.arguments[0] !== undefined &&
          ts.isIdentifier(expression.arguments[0]) &&
          expression.arguments[0].text === caught.text,
      ),
    "loadCatalogue returns no diagnostic derived from the caught error",
  );

  const boot = functionDeclaration(main, "boot");
  assert.ok(boot, "boot is gone from main.ts");
  let assigned: ts.VariableDeclaration | undefined;
  let statusUse: ts.CallExpression | undefined;
  const mentionsCatalogueError = (node: ts.Node): boolean => {
    let found = false;
    walk(node, (candidate) => {
      if (ts.isIdentifier(candidate) && candidate.text === "catalogueError") {
        found = true;
      }
    });
    return found;
  };
  walk(boot, (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === "catalogueError" &&
      node.initializer &&
      ts.isAwaitExpression(node.initializer) &&
      ts.isCallExpression(node.initializer.expression) &&
      ts.isIdentifier(node.initializer.expression.expression) &&
      node.initializer.expression.expression.text === "loadCatalogue"
    ) {
      assigned = node;
    }
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "setStatus" &&
      node.arguments.some(mentionsCatalogueError)
    ) {
      statusUse = node;
    }
  });
  assert.ok(assigned, "boot does not await the catalogue diagnostic");
  assert.ok(
    statusUse && statusUse.getStart() > assigned.getEnd(),
    "boot never uses the catalogue diagnostic in its final status",
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
