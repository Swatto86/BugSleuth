/** Every rebuilt matrix control owns its own keyboard-focus restoration key. */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));

function focusKeyCoverage(
  fileName: string,
  text: string,
): { created: number; missing: string[] } {
  const source = ts.createSourceFile(
    fileName,
    text,
    ts.ScriptTarget.ESNext,
    true,
  );
  const controls: ts.VariableDeclaration[] = [];
  const collect = (node: ts.Node): void => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer &&
      ts.isCallExpression(node.initializer) &&
      ts.isPropertyAccessExpression(node.initializer.expression) &&
      ts.isIdentifier(node.initializer.expression.expression) &&
      node.initializer.expression.expression.text === "document" &&
      node.initializer.expression.name.text === "createElement"
    ) {
      const tag = node.initializer.arguments[0];
      if (
        tag &&
        ts.isStringLiteral(tag) &&
        ["input", "select", "button"].includes(tag.text)
      ) {
        controls.push(node);
      }
    }
    ts.forEachChild(node, collect);
  };
  collect(source);

  const missing: string[] = [];
  for (const declaration of controls) {
    const name = (declaration.name as ts.Identifier).text;
    let block: ts.Node | undefined = declaration.parent;
    while (block && !ts.isBlock(block)) block = block.parent;
    assert.ok(block, `${fileName}: ${name} has no enclosing builder block`);

    let keyed = false;
    const findKey = (node: ts.Node): void => {
      if (
        node.getStart(source) > declaration.getEnd() &&
        ts.isBinaryExpression(node) &&
        node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
        ts.isElementAccessExpression(node.left) &&
        ts.isPropertyAccessExpression(node.left.expression) &&
        ts.isIdentifier(node.left.expression.expression) &&
        node.left.expression.expression.text === name &&
        node.left.expression.name.text === "dataset" &&
        node.left.argumentExpression &&
        ts.isStringLiteral(node.left.argumentExpression) &&
        node.left.argumentExpression.text === "focusKey"
      ) {
        keyed = true;
      }
      ts.forEachChild(node, findKey);
    };
    findKey(block);
    if (!keyed) missing.push(`${fileName}: ${name}`);
  }
  return { created: controls.length, missing };
}

test("every control the matrix rebuilds can be found again afterwards", () => {
  // The row is assembled across both files. Reading only one silently shrinks
  // the scan whenever a picker moves across that seam.
  const coverage = ["view.ts", "pickers.ts"].map((name) =>
    focusKeyCoverage(name, fs.readFileSync(path.join(here, name), "utf8")),
  );
  const created = coverage.reduce((sum, file) => sum + file.created, 0);
  assert.ok(
    created >= 6,
    `found only ${created} focusable controls in the row builder`,
  );
  assert.deepEqual(
    coverage.flatMap((file) => file.missing),
    [],
    "these rebuilt controls have no focus-restoration key",
  );

  const duplicateFixture = focusKeyCoverage(
    "fixture.ts",
    `{
      const first = document.createElement("input");
      const second = document.createElement("button");
      first.dataset["focusKey"] = "one";
      first.dataset["focusKey"] = "duplicate";
    }`,
  );
  assert.deepEqual(duplicateFixture, {
    created: 2,
    missing: ["fixture.ts: second"],
  });
});
