/**
 * Tagging a release is the one control in this window whose consequence leaves
 * the machine entirely: it starts a CI run that can build and publish artifacts
 * other people download. It is only meaningful after a push, so the window must
 * never leave it armed while pushing is off — a stored `true` behind a disabled
 * box is how a release gets cut the next time someone re-enables pushing.
 *
 * The behaviour is exercised for real by the Rust ordering guard
 * (`apply::tests::only_a_push_that_succeeded_can_lead_to_a_tag`), which refuses
 * to tag anything the push did not publish. This reads the frontend's own syntax
 * so the window cannot silently disagree with it.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { frontendFiles, walk } from "./ast.test.ts";

/** A named `const x = (...) => ...` declaration. */
function declaration(
  source: ts.SourceFile,
  name: string,
): ts.ArrowFunction | undefined {
  let found: ts.ArrowFunction | undefined;
  walk(source, (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === name &&
      node.initializer &&
      ts.isArrowFunction(node.initializer)
    ) {
      found ??= node.initializer;
    }
  });
  return found;
}

test("the window never leaves a release armed while pushing is off", () => {
  const files = frontendFiles();
  const apply = files.find((file) => file.fileName === "apply.ts");
  assert.ok(apply, "apply.ts is no longer a shipped frontend module");

  // Found first, and asserted to exist, so a rename cannot turn every check
  // below into one that passes over nothing.
  const drawTag = declaration(apply, "drawTag");
  assert.ok(drawTag, "apply.ts no longer declares drawTag");

  let guard: ts.IfStatement | undefined;
  walk(drawTag, (node) => {
    if (ts.isIfStatement(node)) guard ??= node;
  });
  assert.ok(guard, "drawTag no longer guards release disarming");
  assert.ok(
    ts.isPrefixUnaryExpression(guard.expression) &&
      guard.expression.operator === ts.SyntaxKind.ExclamationToken &&
      ts.isIdentifier(guard.expression.operand) &&
      guard.expression.operand.text === "publishing",
    `drawTag disarms under the wrong condition: ${guard.expression.getText(apply)}`,
  );

  const assignment = (
    node: ts.Node,
    left: string,
  ): ts.BinaryExpression | undefined => {
    let found: ts.BinaryExpression | undefined;
    walk(node, (candidate) => {
      if (
        ts.isBinaryExpression(candidate) &&
        candidate.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
        candidate.left.getText(apply) === left
      ) {
        found ??= candidate;
      }
    });
    return found;
  };
  const disarm = assignment(guard.thenStatement, "live.tag_release_after_push");
  assert.ok(
    disarm && disarm.right.kind === ts.SyntaxKind.FalseKeyword,
    "the not-publishing branch no longer disarms the release",
  );

  const disabled = assignment(drawTag, "ui.tag.disabled");
  assert.ok(
    disabled &&
      ts.isPrefixUnaryExpression(disabled.right) &&
      disabled.right.operator === ts.SyntaxKind.ExclamationToken &&
      ts.isIdentifier(disabled.right.operand) &&
      disabled.right.operand.text === "publishing",
    "the tag control is no longer disabled when pushing is off",
  );
  const checked = assignment(drawTag, "ui.tag.checked");
  assert.ok(
    checked && checked.right.getText(apply) === "live.tag_release_after_push",
    "the tag checkbox no longer renders from the stored setting",
  );
});

test("turning pushing off redraws the tag control before the settings are saved", () => {
  const files = frontendFiles();
  const apply = files.find((file) => file.fileName === "apply.ts");
  assert.ok(apply, "apply.ts is no longer a shipped frontend module");

  // The push handler, located by the setting it writes rather than by position.
  let handler: string | undefined;
  walk(apply, (node) => {
    if (
      ts.isCallExpression(node) &&
      node.getText().includes("push_after_apply = ui.push.checked")
    ) {
      handler ??= node.getText();
    }
  });
  assert.ok(handler, "apply.ts no longer has a push checkbox handler");

  // Order matters: drawTag clears the tag setting, so a save that ran first
  // would persist an armed release the window is about to disarm.
  const redraw = handler.indexOf("drawTag()");
  const save = handler.indexOf("deps.refresh()");
  assert.notEqual(
    redraw,
    -1,
    "the push handler no longer redraws the tag control",
  );
  assert.notEqual(save, -1, "the push handler no longer saves the settings");
  assert.ok(
    redraw < save,
    "the settings are saved before the tag control is disarmed, so an armed release is persisted",
  );
});
