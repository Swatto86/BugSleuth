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

/** A named property's initializer in a call's object literal argument. */
function propertyExpression(
  call: ts.CallExpression,
  property: string,
): ts.Expression | undefined {
  const argument = call.arguments[0];
  if (!argument || !ts.isObjectLiteralExpression(argument)) return undefined;
  for (const member of argument.properties) {
    if (
      ts.isPropertyAssignment(member) &&
      ts.isIdentifier(member.name) &&
      member.name.text === property
    ) {
      return member.initializer;
    }
    // A shorthand `{ busy }` names a binding of the same name.
    if (
      ts.isShorthandPropertyAssignment(member) &&
      member.name.text === property
    ) {
      return member.name;
    }
  }
  return undefined;
}

const propertyInitializer = (call: ts.CallExpression, property: string) =>
  propertyExpression(call, property)?.getText();

function orCalls(expression: ts.Expression): string[] | undefined {
  while (ts.isParenthesizedExpression(expression))
    expression = expression.expression;
  if (
    ts.isCallExpression(expression) &&
    ts.isIdentifier(expression.expression) &&
    expression.arguments.length === 0
  ) {
    return [expression.expression.text];
  }
  if (
    ts.isBinaryExpression(expression) &&
    expression.operatorToken.kind === ts.SyntaxKind.BarBarToken
  ) {
    const left = orCalls(expression.left);
    const right = orCalls(expression.right);
    return left && right ? [...left, ...right] : undefined;
  }
  return undefined;
}

function predicateCalls(
  call: ts.CallExpression,
  property: string,
): string[] | undefined {
  const initializer = propertyExpression(call, property);
  return initializer &&
    ts.isArrowFunction(initializer) &&
    ts.isExpression(initializer.body)
    ? orCalls(initializer.body)
    : undefined;
}

function methodCalls(source: ts.SourceFile, name: string): ts.CallExpression[] {
  const found: ts.CallExpression[] = [];
  walk(source, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === name
    ) {
      found.push(node);
    }
  });
  return found;
}

test("the updater is blocked while a review or an apply is in flight", () => {
  const main = frontendFiles().find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");

  // Assert the call was found first, so a rename cannot make this pass vacuously.
  const calls = callsTo(main, "wireUpdate");
  assert.ok(calls.length >= 1, "wireUpdate is no longer called from main.ts");

  for (const call of calls) {
    assert.deepEqual(
      predicateCalls(call, "busy")?.sort(),
      ["isApplying", "isClearing", "isRunning"],
      `the updater's busy predicate is not exactly three ORed activity checks: ${propertyInitializer(call, "busy")}`,
    );
  }
});

test("the updater busy predicate rejects conjunctions", () => {
  const source = ts.createSourceFile(
    "fixture.ts",
    "wireUpdate({ busy: () => (isRunning() && isApplying()) || isClearing() });",
    ts.ScriptTarget.ESNext,
    true,
  );
  const call = callsTo(source, "wireUpdate")[0];
  assert.ok(call, "the malformed fixture did not parse");
  assert.equal(predicateCalls(call, "busy"), undefined);
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
  const confirm = confirms[0]!;

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

  const applyCall = callsTo(main, "bindApply")[0];
  assert.ok(applyCall, "bindApply is no longer called from main.ts");
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

test("installing an update locks and saves settings before restart", () => {
  const files = frontendFiles();
  const main = files.find((file) => file.fileName === "main.ts");
  const update = files.find((file) => file.fileName === "update.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");
  assert.ok(update, "update.ts is no longer a shipped frontend module");

  const wiring = callsTo(main, "wireUpdate");
  assert.equal(wiring.length, 1, "wireUpdate must have one production caller");
  assert.ok(
    propertyExpression(wiring[0]!, "flushSettings"),
    "wireUpdate is not given the settings flush",
  );
  assert.ok(
    propertyExpression(wiring[0]!, "setSettingsLocked"),
    "wireUpdate is not given the settings lock",
  );

  const confirm = callsTo(update, "confirmDialog")[0];
  const install = callsTo(update, "invoke").find(
    (call) => stringArgument(call) === "install_update",
  );
  const flush = methodCalls(update, "flushSettings")[0];
  const locks = methodCalls(update, "setSettingsLocked");
  const lock = locks.find(
    (call) => call.arguments[0]?.kind === ts.SyntaxKind.TrueKeyword,
  );
  const unlock = locks.find(
    (call) => call.arguments[0]?.kind === ts.SyntaxKind.FalseKeyword,
  );
  assert.ok(confirm, "the install confirmation disappeared");
  assert.ok(install, "the real install_update invocation disappeared");
  assert.ok(flush, "settings are not flushed before update restart");
  assert.ok(lock, "settings controls are not locked during installation");
  assert.ok(unlock, "settings controls are not unlocked after install failure");
  assert.ok(
    ts.isAwaitExpression(flush.parent),
    "the settings flush is started but not awaited",
  );
  assert.ok(confirm.getStart(update) < lock.getStart(update));
  assert.ok(lock.getStart(update) < flush.getStart(update));
  assert.ok(flush.getStart(update) < install.getStart(update));
  assert.ok(install.getStart(update) < unlock.getStart(update));
});
