/** The native E2E driver is checked before launch and always cleaned up. */

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

test("the E2E harness waits for its driver cleanup and rejects a stale port", () => {
  const text = fs.readFileSync(path.join(root, "e2e", "wdio.conf.ts"), "utf8");
  const source = parse("wdio.conf.ts", text);
  let config: ts.ObjectLiteralExpression | undefined;
  for (const statement of source.statements) {
    if (
      !ts.isVariableStatement(statement) ||
      !statement.modifiers?.some(
        (modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword,
      )
    ) {
      continue;
    }
    const declaration = statement.declarationList.declarations.find(
      (candidate) =>
        ts.isIdentifier(candidate.name) && candidate.name.text === "config",
    );
    if (
      declaration?.initializer &&
      ts.isObjectLiteralExpression(declaration.initializer)
    ) {
      config = declaration.initializer;
    }
  }
  assert.ok(config, "the exported WebdriverIO config object was not found");

  const hook = (name: string) =>
    config.properties.find(
      (property): property is ts.PropertyAssignment =>
        ts.isPropertyAssignment(property) &&
        property.name.getText(source) === name,
    );
  const mochaOpts = hook("mochaOpts");
  assert.ok(
    mochaOpts && ts.isObjectLiteralExpression(mochaOpts.initializer),
    "the E2E Mocha options were not found",
  );
  const timeout = mochaOpts.initializer.properties.find(
    (property): property is ts.PropertyAssignment =>
      ts.isPropertyAssignment(property) &&
      property.name.getText(source) === "timeout",
  );
  assert.equal(
    timeout?.initializer.getText(source),
    "15 * 60_000",
    "the runner timeout cannot cover the real review's 14-minute wait",
  );
  const onPrepare = hook("onPrepare");
  const onComplete = hook("onComplete");
  assert.ok(onPrepare, "the E2E onPrepare hook was not found");
  assert.ok(onComplete, "the E2E onComplete hook was not found");

  let portCheck: ts.CallExpression | undefined;
  let driverSpawn: ts.CallExpression | undefined;
  let driverSpawns = 0;
  walk(source, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "spawn" &&
      node.arguments[0]?.getText(source) === '"tauri-driver"'
    ) {
      driverSpawns += 1;
    }
  });
  walk(onPrepare, (node) => {
    if (!ts.isCallExpression(node) || !ts.isIdentifier(node.expression)) return;
    if (node.expression.text === "assertDriverPortFree") portCheck = node;
    if (
      node.expression.text === "spawn" &&
      node.arguments[0]?.getText(source) === '"tauri-driver"'
    ) {
      driverSpawn = node;
    }
  });
  assert.equal(driverSpawns, 1, "the check found no unique driver launch");
  assert.ok(portCheck, "onPrepare no longer checks for a stale driver");
  assert.ok(driverSpawn, "onPrepare no longer starts tauri-driver");
  assert.ok(ts.isAwaitExpression(portCheck.parent));
  assert.ok(portCheck.parent.getEnd() < driverSpawn.getStart(source));

  const options = driverSpawn.arguments[2];
  assert.ok(options && ts.isObjectLiteralExpression(options));
  const shell = options.properties.find(
    (property) => property.name?.getText(source) === "shell",
  );
  assert.ok(
    shell &&
      ts.isPropertyAssignment(shell) &&
      shell.initializer.kind === ts.SyntaxKind.FalseKeyword,
    "tauri-driver must not be launched through a shell",
  );

  let taskkill: ts.CallExpression | undefined;
  walk(onComplete, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "spawnSync" &&
      node.arguments[0]?.getText(source) === '"taskkill"'
    ) {
      taskkill = node;
    }
  });
  assert.ok(taskkill, "onComplete no longer kills the owned driver tree");
  assert.deepEqual(
    taskkill.arguments[1]?.getText(source),
    '["/pid", String(tauriDriver.pid), "/T", "/F"]',
  );
});
