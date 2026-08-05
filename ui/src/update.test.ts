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

import { callsTo, frontendFiles } from "./ast.test.ts";

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
