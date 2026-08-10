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
import ts from "typescript";
import { frontendFiles, lineOf, walk } from "./ast.test.ts";

function enclosingFunction(
  node: ts.Node,
): ts.FunctionLikeDeclaration | undefined {
  for (let parent = node.parent; parent; parent = parent.parent) {
    if (
      ts.isArrowFunction(parent) ||
      ts.isFunctionDeclaration(parent) ||
      ts.isFunctionExpression(parent) ||
      ts.isMethodDeclaration(parent)
    ) {
      return parent;
    }
  }
  return undefined;
}

function handsFocusOffBefore(scope: ts.Node, position: number): boolean {
  let found = false;
  walk(scope, (node) => {
    if (!ts.isCallExpression(node) || node.getStart() >= position) return;
    const callee = node.expression;
    if (
      ts.isPropertyAccessExpression(callee) &&
      callee.name.text === "focusStatus"
    ) {
      found = true;
    }
  });
  return found;
}

test("starting a run moves focus off the disabled Run button to Stop", () => {
  const run = frontendFiles().find((f) => f.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  assert.match(
    run.getText(),
    /deps\.stop\.focus\(\)/,
    "startRun never moves focus to Stop, so activating Run drops focus to <body>",
  );
});

test("every active control hands focus off before it disables or hides itself", () => {
  const files = frontendFiles();
  const assignments: { source: ts.SourceFile; node: ts.BinaryExpression }[] =
    [];
  for (const source of files) {
    walk(source, (node) => {
      if (
        ts.isBinaryExpression(node) &&
        node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
        node.right.kind === ts.SyntaxKind.TrueKeyword &&
        ts.isPropertyAccessExpression(node.left) &&
        node.left.name.text === "disabled"
      ) {
        assignments.push({ source, node });
      }
    });
  }

  const picker = assignments.filter(
    ({ source }) => source.fileName === "pickers.ts",
  );
  assert.equal(
    picker.length,
    1,
    "the deliberately born-disabled picker vanished",
  );
  const active = assignments.filter(
    ({ source }) => source.fileName !== "pickers.ts",
  );
  assert.deepEqual(
    active
      .map(
        ({ source, node }) => `${source.fileName}:${node.left.getText(source)}`,
      )
      .sort(),
    [
      "actions.ts:ui.clearSaved.disabled",
      "actions.ts:ui.stop.disabled",
      "apply.ts:deps.ui.button.disabled",
      "controls.ts:ui.checkSignin.disabled",
      "update.ts:button.disabled",
    ],
  );
  for (const { source, node } of active) {
    const handler = enclosingFunction(node);
    assert.ok(
      handler,
      `${source.fileName}:${lineOf(source, node)} has no handler`,
    );
    assert.ok(
      handsFocusOffBefore(handler, node.getStart()),
      `${source.fileName}:${lineOf(source, node)} disables an active control before handing focus off`,
    );
  }

  const run = files.find((source) => source.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  const endingRenders: ts.CallExpression[] = [];
  walk(run, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "renderPlanSummary"
    ) {
      endingRenders.push(node);
    }
  });
  assert.equal(
    endingRenders.length,
    3,
    "run.ts no longer has its three lifecycle renders",
  );
  assert.equal(
    endingRenders.filter((call) => {
      const handler = enclosingFunction(call);
      return handler && handsFocusOffBefore(handler, call.getStart());
    }).length,
    2,
    "failed and finished runs hide Stop without first handing focus off",
  );
});

test("a refused start restores the findings it cleared", () => {
  const run = frontendFiles().find((source) => source.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  const catches: ts.CatchClause[] = [];
  walk(run, (node) => {
    if (ts.isCatchClause(node)) catches.push(node);
  });
  assert.equal(catches.length, 1, "startRun no longer has one refusal path");

  let restoresCards = false;
  let restoresApply = false;
  walk(catches[0]!.block, (node) => {
    if (
      !ts.isCallExpression(node) ||
      !ts.isPropertyAccessExpression(node.expression)
    ) {
      return;
    }
    const callee = node.expression;
    if (
      callee.name.text === "replaceChildren" &&
      node.arguments.some(
        (argument) =>
          ts.isSpreadElement(argument) &&
          ts.isIdentifier(argument.expression) &&
          argument.expression.text === "previousCards",
      )
    ) {
      restoresCards = true;
    }
    if (
      callee.name.text === "remove" &&
      node.arguments.some(
        (argument) =>
          ts.isStringLiteral(argument) && argument.text === "hidden",
      )
    ) {
      restoresApply = true;
    }
  });
  assert.ok(
    restoresCards,
    "the refusal path never restores the previous cards",
  );
  assert.ok(restoresApply, "the refusal path never restores the Apply panel");
});

test("a new run withdraws the previous run's prompt controls", () => {
  const run = frontendFiles().find((f) => f.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  assert.match(
    run.getText(),
    /deps\.copyPrompt\.classList\.add\("hidden"\)/,
    "startRun leaves the previous run's Copy fix prompt button offered",
  );
  assert.match(
    run.getText(),
    /deps\.promptPath\.classList\.add\("hidden"\)/,
    "startRun leaves the previous run's saved-path note showing",
  );
});

test("a new run withdraws the previous run's report action", () => {
  const run = frontendFiles().find((f) => f.fileName === "run.ts");
  assert.ok(run, "run.ts is no longer a shipped frontend module");
  assert.match(
    run.getText(),
    /deps\.copyReport\.classList\.add\("hidden"\)/,
    "startRun leaves the previous run's Copy report button offered",
  );
  assert.match(
    run.getText(),
    /deps\.copyReport\.classList\.toggle\("hidden",\s*currentReport === ""\)/,
    "a finished run never offers its report for copying",
  );
});

/// Focus has to leave Clear before anything can disable it.
///
/// The confirmation dialog restores focus to Clear saved sweeps before
/// resolving. `activityChanged()` then reaches `renderPlanSummary`, which
/// disables that focused button — and WebView2 drops focus to `<body>`, so the
/// `document.activeElement === ui.clearSaved` check below it was already false
/// and the live status region never received focus. The keyboard user was left
/// at the document root for the whole delete.
test("clearing hands focus off before activityChanged disables its opener", () => {
  const actions = frontendFiles().find(
    (source) => source.fileName === "actions.ts",
  );
  assert.ok(actions, "actions.ts is no longer a shipped frontend module");

  let handler: ts.ArrowFunction | undefined;
  walk(actions, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "addEventListener" &&
      ts.isPropertyAccessExpression(node.expression.expression) &&
      node.expression.expression.name.text === "clearSaved"
    ) {
      const candidate = node.arguments[1];
      if (candidate && ts.isArrowFunction(candidate)) handler = candidate;
    }
  });
  assert.ok(handler, "the Clear saved sweeps handler is gone");

  let focus = -1;
  let activity = -1;
  walk(handler.body, (node) => {
    if (
      !ts.isCallExpression(node) ||
      !ts.isPropertyAccessExpression(node.expression)
    )
      return;
    if (node.expression.name.text === "focusStatus") {
      focus = node.getStart(actions);
    }
    if (node.expression.name.text === "activityChanged" && activity < 0) {
      activity = node.getStart(actions);
    }
  });
  assert.ok(focus >= 0, "the clear handler never hands focus off");
  assert.ok(activity >= 0, "the clear handler never updates global activity");
  assert.ok(
    focus < activity,
    "activityChanged disables Clear before focus leaves it",
  );
});
