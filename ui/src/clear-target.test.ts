/** Clearing one repository must not hide another repository's Apply action. */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const read = (name: string): string =>
  fs.readFileSync(path.join(here, name), "utf8");

const parse = (name: string): ts.SourceFile =>
  ts.createSourceFile(name, read(name), ts.ScriptTarget.ESNext, true);

const walk = (node: ts.Node, visit: (node: ts.Node) => void): void => {
  visit(node);
  node.forEachChild((child) => walk(child, visit));
};

test("clearing a different repository keeps the displayed Apply action", () => {
  const source = parse("actions.ts");
  let callback: ts.ArrowFunction | undefined;
  walk(source, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "then" &&
      node.expression.expression.getText().includes('"clear_saved"')
    ) {
      const candidate = node.arguments[0];
      if (candidate && ts.isArrowFunction(candidate)) callback = candidate;
    }
  });
  assert.ok(callback, "the clear_saved success callback was not found");

  const hides = new Map<string, ts.CallExpression>();
  let guard: ts.IfStatement | undefined;
  walk(callback.body, (node) => {
    if (
      ts.isCallExpression(node) &&
      /ui\.(applyPanel|promptPath)\.classList\.add\("hidden"\)/.test(
        node.getText(),
      )
    ) {
      hides.set(node.getText().includes("applyPanel") ? "apply" : "path", node);
    }
    if (
      ts.isIfStatement(node) &&
      node.expression.getText() ===
        "cleared.promptPath === deps.currentPromptPath()"
    ) {
      guard = node;
    }
  });

  assert.ok(hides.has("apply"), "the Apply-panel hide call was not found");
  assert.ok(hides.has("path"), "the prompt-path hide call was not found");
  assert.ok(guard, "the prompt paths are not compared before hiding");
  for (const hide of hides.values()) {
    assert.ok(
      hide.getStart() >= guard.thenStatement.getStart() &&
        hide.getEnd() <= guard.thenStatement.getEnd(),
      `${hide.getText()} is not guarded by the matching prompt path`,
    );
  }

  assert.match(read("run.ts"), /export const currentFixPromptPath/);
  assert.match(read("main.ts"), /currentPromptPath:\s*currentFixPromptPath/);
});
