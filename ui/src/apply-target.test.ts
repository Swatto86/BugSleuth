import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

import { parse, walk } from "./ast.test.ts";
import { settingsForApply, type Settings } from "./model.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const read = (name: string) => fs.readFileSync(path.join(here, name), "utf8");

test("applying uses the repository that produced the displayed prompt", () => {
  const live: Settings = {
    repo: "C:/repo-b",
    scope: "",
    models: [],
    theme: "system",
    reuse_completed: true,
    triage_model: "",
    apply_model: "sonnet",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  };
  const sent = settingsForApply(live, "C:/repo-a");
  assert.equal(sent.repo, "C:/repo-a");
  assert.equal(
    live.repo,
    "C:/repo-b",
    "building the request mutated live settings",
  );

  const run = read("run.ts");
  const apply = read("apply.ts");
  const main = read("main.ts");
  assert.match(run, /currentFixPromptRepo/);
  assert.match(main, /promptRepo:\s*currentFixPromptRepo/);

  const source = parse("apply.ts", apply);
  let start: ts.FunctionDeclaration | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "start") {
      start = node;
    }
  });
  assert.ok(start, "the Apply start function disappeared");

  let invocation: ts.CallExpression | undefined;
  let declaration: ts.VariableDeclaration | undefined;
  walk(start, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      node.expression.text === "invoke" &&
      node.arguments[0] &&
      ts.isStringLiteral(node.arguments[0]) &&
      node.arguments[0].text === "apply_fixes"
    ) {
      invocation = node;
    }
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === "settings"
    ) {
      declaration = node;
    }
  });
  assert.ok(invocation, "the real Apply command disappeared");
  const payload = invocation.arguments[1];
  assert.ok(payload && ts.isObjectLiteralExpression(payload));
  assert.ok(
    payload.properties.some(
      (property) =>
        ts.isShorthandPropertyAssignment(property) &&
        property.name.text === "settings",
    ),
    "Apply does not send the repository-bound settings",
  );
  assert.ok(
    declaration,
    "the repository-bound settings declaration disappeared",
  );
  assert.ok(
    declaration.getStart(source) < invocation.getStart(source),
    "settings are declared after the invocation",
  );
  assert.ok(
    (declaration.parent.flags & ts.NodeFlags.Const) !== 0,
    "the repository-bound settings are no longer constant",
  );
  assert.equal(
    declaration.initializer?.getText(source),
    "settingsForApply(deps.settings(), repo)",
  );
});
