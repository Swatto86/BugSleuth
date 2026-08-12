/**
 * The updater restarts the process, so it must be blocked while any process-
 * bound operation is in flight — not just a review, but an apply that edits the
 * repository in place. This project shipped a busy predicate that consulted
 * only `isRunning`, so pressing Install mid-apply killed the model part-way
 * through and left the repository half-changed.
 *
 * Read main.ts's own syntax rather than trusting the comment beside the call:
 * the predicate must reference both `isRunning` and `isApplying`.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { callsTo, frontendFiles, stringArgument, walk } from "./ast.test.ts";

/** The source text of a named property's initializer in a call's object
 * literal argument, or undefined if the call has no such property. */
function propertyInitializer(
  call: ts.CallExpression,
  property: string,
): string | undefined {
  const argument = call.arguments[0];
  if (!argument || !ts.isObjectLiteralExpression(argument)) return undefined;
  for (const member of argument.properties) {
    if (
      ts.isPropertyAssignment(member) &&
      ts.isIdentifier(member.name) &&
      member.name.text === property
    ) {
      return member.initializer.getText();
    }
    // A shorthand `{ busy }` names a binding of the same name.
    if (
      ts.isShorthandPropertyAssignment(member) &&
      member.name.text === property
    ) {
      return member.name.text;
    }
  }
  return undefined;
}

test("the updater is blocked while a review or an apply is in flight", () => {
  const main = frontendFiles().find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");

  // Assert the call was found first, so a rename cannot make this pass vacuously.
  const calls = callsTo(main, "wireUpdate");
  assert.ok(calls.length >= 1, "wireUpdate is no longer called from main.ts");

  for (const call of calls) {
    const busy = propertyInitializer(call, "busy");
    assert.ok(busy, "wireUpdate is called without a busy predicate");
    assert.match(
      busy,
      /isRunning/,
      `the updater's busy predicate does not consult isRunning: ${busy}`,
    );
    assert.match(
      busy,
      /isApplying/,
      `the updater's busy predicate does not consult isApplying, so an install ` +
        `could restart the process mid-apply: ${busy}`,
    );
  }
});

test("declining an available update clears the running status", () => {
  const update = frontendFiles().find((file) => file.fileName === "update.ts");
  assert.ok(update, "update.ts is no longer a shipped frontend module");

  let declined: ts.IfStatement | undefined;
  walk(update, (node) => {
    if (
      ts.isIfStatement(node) &&
      node.expression.getText(update) === "!agreed"
    ) {
      declined = node;
    }
  });
  assert.ok(declined, "the update confirmation has no declined path");
  assert.match(
    declined.thenStatement.getText(update),
    /setStatus\s*\(/,
    "declining the install leaves the running status and spinner stuck",
  );
});

test("an available update clears the checking status before the install confirmation", () => {
  const update = frontendFiles().find((file) => file.fileName === "update.ts");
  assert.ok(update, "update.ts is no longer a shipped frontend module");

  let busyGuard: ts.IfStatement | undefined;
  walk(update, (node) => {
    if (
      ts.isIfStatement(node) &&
      node.expression.getText(update) === "deps.busy()"
    ) {
      busyGuard ??= node;
    }
  });
  assert.ok(busyGuard, "the update handler has no busy guard before confirm");

  const confirms = callsTo(update, "confirmDialog");
  assert.ok(
    confirms.length >= 1,
    "confirmDialog is no longer called from update.ts",
  );
  const confirm = confirms[0];

  const statusCalls = callsTo(update, "setStatus").filter(
    (call) =>
      call.getStart(update) > busyGuard!.getEnd() &&
      call.getStart(update) < confirm.getStart(update),
  );
  assert.ok(
    statusCalls.length >= 1,
    "no setStatus runs after the busy guard and before confirmDialog, so the checking spinner stays up during the install prompt",
  );
  for (const call of statusCalls) {
    const kind = call.arguments[1]?.getText(update) ?? "";
    assert.notEqual(
      kind,
      '"running"',
      `pre-confirm setStatus still uses the running kind: ${call.getText(update)}`,
    );
  }
});

test("installing an update disables every operation its restart would interrupt", () => {
  const files = frontendFiles();
  const main = files.find((file) => file.fileName === "main.ts");
  const update = files.find((file) => file.fileName === "update.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");
  assert.ok(update, "update.ts is no longer a shipped frontend module");

  const updateCall = callsTo(main, "wireUpdate")[0];
  const applyCall = callsTo(main, "bindApply")[0];
  assert.ok(updateCall, "wireUpdate is no longer called from main.ts");
  assert.ok(applyCall, "bindApply is no longer called from main.ts");
  assert.match(propertyInitializer(updateCall, "busy") ?? "", /isClearing/);
  assert.match(propertyInitializer(applyCall, "busy") ?? "", /isUpdating/);

  let renderPlanSummary = "";
  walk(main, (node) => {
    if (
      ts.isFunctionDeclaration(node) &&
      node.name?.text === "renderPlanSummary"
    ) {
      renderPlanSummary = node.getText(main);
    }
  });
  assert.ok(renderPlanSummary, "renderPlanSummary was not found");
  assert.match(renderPlanSummary, /isUpdating/);

  const install = callsTo(update, "invoke").find(
    (call) => stringArgument(call) === "install_update",
  );
  assert.ok(install, "the real install_update invocation disappeared");
  const assignments: Array<{ value: boolean; position: number }> = [];
  walk(update, (node) => {
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isIdentifier(node.left) &&
      node.left.text === "updating" &&
      (node.right.kind === ts.SyntaxKind.TrueKeyword ||
        node.right.kind === ts.SyntaxKind.FalseKeyword)
    ) {
      assignments.push({
        value: node.right.kind === ts.SyntaxKind.TrueKeyword,
        position: node.getStart(update),
      });
    }
  });
  assert.ok(
    assignments.some(
      ({ value, position }) => value && position < install.getStart(update),
    ),
    "updating is not set before install_update starts",
  );
  assert.ok(
    assignments.some(
      ({ value, position }) => !value && position > install.getStart(update),
    ),
    "updating is not cleared after install_update rejects",
  );
});
