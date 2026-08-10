/**
 * Boot and catalogue loads must not persist the settings they render.
 *
 * When the saved file is unreadable the app shows defaults; if that initial
 * render wrote them back it would overwrite the recoverable file. This guards
 * the two ends of the fix: `refresh` only persists when not suppressed, and the
 * two startup renders go through `renderWithoutPersisting`.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("startup renders do not persist settings back over the saved file", () => {
  const main = read("ui", "src", "main.ts");

  assert.ok(
    main.includes("function renderWithoutPersisting"),
    "the non-persisting render helper is gone",
  );
  assert.ok(
    main.includes("if (!suppressPersistence) persist()"),
    "refresh no longer gates persistence, so a startup render writes defaults back",
  );

  const source = ts.createSourceFile(
    "main.ts",
    main,
    ts.ScriptTarget.ESNext,
    true,
  );
  const walk = (node: ts.Node, visit: (candidate: ts.Node) => void): void => {
    visit(node);
    node.forEachChild((child) => walk(child, visit));
  };
  for (const name of ["boot", "loadCatalogue"]) {
    let owner: ts.FunctionDeclaration | undefined;
    walk(source, (node) => {
      if (ts.isFunctionDeclaration(node) && node.name?.text === name)
        owner = node;
    });
    assert.ok(owner?.body, `${name} is gone from main.ts`);
    let calls = 0;
    walk(owner.body, (node) => {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        node.expression.text === "renderWithoutPersisting"
      ) {
        calls += 1;
      }
    });
    assert.equal(calls, 1, `${name} must render once without persisting`);
  }
});
