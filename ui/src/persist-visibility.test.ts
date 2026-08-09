/**
 * A settings-save failure must be visible even during a run.
 *
 * The old handler suppressed the error whenever a run was in progress — exactly
 * when a save is most likely to fail — so a whole run's configuration could be
 * lost silently. The failure now goes to its own alert region, ungated. This
 * guards that the gate is gone and the region exists.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

import type { Settings } from "./model.ts";
import { savingSettings } from "./persist.ts";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("a save failure is shown in its own region, never gated on a run", () => {
  const persist = read("ui", "src", "persist.ts");
  assert.ok(
    persist.includes("deps.setError("),
    "the save handler no longer reports through a dedicated error region",
  );
  assert.ok(
    !/quiet/.test(persist),
    "the save error is still suppressed while a run is in progress",
  );

  const html = read("ui", "index.html");
  assert.match(
    html,
    /id="settings-error"[^>]*role="alert"/,
    "the settings-error alert region is missing from the footer",
  );
});

test("flushing writes the latest pending settings exactly once", async () => {
  const settings: Settings = {
    repo: "old",
    scope: "",
    models: [],
    theme: "system",
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
  };
  const saved: Settings[] = [];
  const saver = savingSettings({
    settings: () => settings,
    setError: () => undefined,
    save: async (snapshot) => {
      saved.push(snapshot);
    },
  });

  saver.schedule();
  settings.repo = "latest";
  assert.equal(await saver.flush(), true);
  await new Promise((resolve) => globalThis.setTimeout(resolve, 450));
  assert.deepEqual(
    saved.map((snapshot) => snapshot.repo),
    ["latest"],
  );
});

test("quit flushes pending settings before it can invoke exit", () => {
  const source = ts.createSourceFile(
    "actions.ts",
    read("ui", "src", "actions.ts"),
    ts.ScriptTarget.ESNext,
    true,
  );
  let requestQuit: ts.FunctionDeclaration | undefined;
  const walk = (node: ts.Node): void => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "requestQuit") {
      requestQuit = node;
    }
    node.forEachChild(walk);
  };
  walk(source);
  assert.ok(
    requestQuit,
    "actions.ts no longer has the shared requestQuit path",
  );

  let flush = -1;
  let quit = -1;
  const inspect = (node: ts.Node): void => {
    if (ts.isCallExpression(node)) {
      const callee = node.expression;
      if (
        ts.isPropertyAccessExpression(callee) &&
        callee.name.text === "flushSettings"
      ) {
        flush = node.getStart(source);
      }
      if (
        ts.isIdentifier(callee) &&
        callee.text === "invoke" &&
        node.arguments[0] !== undefined &&
        ts.isStringLiteral(node.arguments[0]) &&
        node.arguments[0].text === "quit"
      ) {
        quit = node.getStart(source);
      }
    }
    node.forEachChild(inspect);
  };
  inspect(requestQuit);
  assert.ok(flush >= 0, "requestQuit never flushes settings");
  assert.ok(quit >= 0, "requestQuit no longer reaches the quit command");
  assert.ok(flush < quit, "requestQuit can exit before its settings flush");
});
