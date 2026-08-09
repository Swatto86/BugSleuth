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

  // Exactly the two startup call sites — boot and loadCatalogue — render this
  // way; the definition uses the name without a call.
  const calls = (main.match(/renderWithoutPersisting\(\)/g) ?? []).length;
  assert.ok(
    calls >= 2,
    `boot and loadCatalogue should render without persisting, found ${calls} call(s)`,
  );
});

test("stored provider concurrency is clamped at boot", () => {
  const main = ts.createSourceFile(
    "main.ts",
    read("ui", "src", "main.ts"),
    ts.ScriptTarget.ESNext,
    true,
  );
  let boot: ts.FunctionDeclaration | undefined;
  const findBoot = (node: ts.Node): void => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "boot")
      boot = node;
    node.forEachChild(findBoot);
  };
  findBoot(main);
  assert.ok(boot, "boot() is gone from main.ts");

  let clamps = false;
  const inspect = (node: ts.Node): void => {
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isPropertyAccessExpression(node.left) &&
      node.left.expression.getText(main) === "settings" &&
      node.left.name.text === "provider_concurrency" &&
      ts.isCallExpression(node.right) &&
      ts.isIdentifier(node.right.expression) &&
      node.right.expression.text === "boundedProviderConcurrency"
    ) {
      clamps = true;
    }
    node.forEachChild(inspect);
  };
  inspect(boot);
  assert.ok(
    clamps,
    "boot shows stored concurrency without the bound the run enforces",
  );
});
