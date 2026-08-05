/**
 * Removing a configured model row destroys a provider, model id, effort, pass
 * count and five lane assignments at once, with no undo. Like preset
 * replacement and clearing saved sweeps, it must pass through a confirmation
 * rather than mutating settings on the spot.
 *
 * The behaviour itself is exercised end-to-end in e2e/specs/review.spec.ts; this
 * reads main.ts's own syntax so a regression is caught without a live app.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { callsTo, frontendFiles } from "./ast.test.ts";

/** The source text of a named handler in any object-literal argument of a call. */
function handlerText(
  call: ts.CallExpression,
  name: string,
): string | undefined {
  for (const argument of call.arguments) {
    if (!ts.isObjectLiteralExpression(argument)) continue;
    for (const member of argument.properties) {
      if (
        (ts.isPropertyAssignment(member) ||
          ts.isMethodDeclaration(member) ||
          ts.isShorthandPropertyAssignment(member)) &&
        ts.isIdentifier(member.name) &&
        member.name.text === name
      ) {
        return member.getText();
      }
    }
  }
  return undefined;
}

test("removing a configured model row is guarded by a confirmation", () => {
  const main = frontendFiles().find((file) => file.fileName === "main.ts");
  assert.ok(main, "main.ts is no longer a shipped frontend module");

  // Assert the call was found first, so a rename cannot make this pass vacuously.
  const calls = callsTo(main, "matrixRows");
  assert.ok(calls.length >= 1, "matrixRows is no longer called from main.ts");

  let checked = 0;
  for (const call of calls) {
    const onRemove = handlerText(call, "onRemove");
    if (!onRemove) continue;
    checked += 1;
    assert.match(
      onRemove,
      /confirmDialog/,
      `the model-row remove handler deletes without confirmation: ${onRemove}`,
    );
  }
  assert.ok(
    checked >= 1,
    "no onRemove handler was found on any matrixRows call",
  );
});
