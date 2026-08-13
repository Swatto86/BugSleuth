/**
 * A preset replace skips its confirmation only when nothing is lost.
 *
 * The guard used to test each row against "is a shipped row", so an arbitrary
 * mix, subset or duplicate of shipped rows passed and a user-built matrix was
 * destroyed without a prompt. The handler must key on the whole configuration.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

import { parse, walk } from "./ast.test.ts";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("the preset handler guards on the whole configuration, not per row", () => {
  const text = read("ui", "src", "actions.ts");
  const actions = parse("actions.ts", text);
  let presetLoop: ts.ForOfStatement | undefined;
  walk(actions, (node) => {
    if (
      ts.isForOfStatement(node) &&
      node.expression.getText(actions).includes('"cheap"')
    ) {
      presetLoop = node;
    }
  });
  assert.ok(presetLoop, "the shared preset loop was not found");

  let handler: ts.ArrowFunction | undefined;
  walk(presetLoop, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "addEventListener" &&
      node.arguments[0]?.getText(actions) === '"click"' &&
      node.arguments[1] !== undefined &&
      ts.isArrowFunction(node.arguments[1])
    ) {
      handler = node.arguments[1];
    }
  });
  assert.ok(handler, "the preset click handler was not found");

  let guard: ts.IfStatement | undefined;
  walk(handler, (node) => {
    if (
      ts.isIfStatement(node) &&
      node.expression.getText(actions).includes("isShippedConfiguration")
    ) {
      guard = node;
    }
  });
  assert.ok(guard, "the preset replacement guard was not found");
  assert.ok(
    ts.isCallExpression(guard.expression),
    `the shipped-configuration guard has the wrong polarity: ${guard.expression.getText(actions)}`,
  );
  assert.equal(
    guard.expression.expression.getText(actions),
    "deps.isShippedConfiguration",
  );
  assert.deepEqual(
    guard.expression.arguments.map((argument) => argument.getText(actions)),
    ["deps.settings().models"],
  );

  assert.ok(ts.isBlock(guard.thenStatement));
  assert.equal(
    guard.thenStatement.statements[0]?.getText(actions),
    "deps.setSettings(preset(name));",
  );
  assert.equal(
    guard.thenStatement.statements[1]?.getText(actions),
    "deps.render();",
  );
  assert.ok(
    guard.thenStatement.statements[2] !== undefined &&
      ts.isReturnStatement(guard.thenStatement.statements[2]),
  );

  let confirm: ts.CallExpression | undefined;
  walk(handler, (node) => {
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(actions) === "confirmDialog"
    ) {
      confirm = node;
    }
  });
  assert.ok(confirm, "the custom-configuration confirmation was not found");
  assert.ok(
    guard.getEnd() < confirm.getStart(actions),
    "confirmation must follow the shipped-preset fast path",
  );
});
