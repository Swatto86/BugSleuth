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

test("a save failure is shown in its own region, never gated on a run", async () => {
  const settings: Settings = {
    repo: "repo",
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
  const errors: string[] = [];
  const saver = savingSettings({
    settings: () => settings,
    setError: (text) => errors.push(text),
    save: async () => {
      throw new Error("disk full");
    },
  });
  saver.allowWrites();
  saver.schedule();
  assert.equal(await saver.flush(), false);
  assert.deepEqual(errors, ["Settings are not being saved: Error: disk full"]);

  const html = read("ui", "index.html");
  assert.match(
    html,
    /id="settings-error"[^>]*role="alert"/,
    "the settings-error alert region is missing from the footer",
  );
});

test("the plan cost is announced only when it changes", () => {
  const html = read("ui", "index.html");
  assert.match(
    html,
    /id="plan-summary"[^>]*role="status"/,
    "the plan summary is not a live region, so cost changes are never announced",
  );

  const source = ts.createSourceFile(
    "main.ts",
    read("ui", "src", "main.ts"),
    ts.ScriptTarget.ESNext,
    true,
  );
  let render: ts.FunctionDeclaration | undefined;
  const walk = (node: ts.Node): void => {
    if (
      ts.isFunctionDeclaration(node) &&
      node.name?.text === "renderPlanSummary"
    ) {
      render = node;
    }
    node.forEachChild(walk);
  };
  walk(source);
  assert.ok(render, "renderPlanSummary is gone from main.ts");
  const text = render.getText(source);
  assert.match(
    text,
    /planSummary\.textContent !== summary/,
    "unchanged plan costs are rewritten and announced again",
  );
  assert.match(text, /planSummary\.textContent = summary/);
  assert.match(
    text,
    /runBlockReason/,
    "Run is still disabled from a boolean with no reason",
  );
  assert.match(
    text,
    /ui\.run\.title/,
    "the Run button has no title explaining why it is disabled",
  );
  assert.match(
    text,
    /ui\.runReason/,
    "a blocked run is not surfaced in its own live region",
  );
  assert.doesNotMatch(
    text,
    /ui\.uncovered/,
    "renderPlanSummary overwrites the independent lane-coverage warning",
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

  saver.allowWrites();
  saver.schedule();
  settings.repo = "latest";
  assert.equal(await saver.flush(), true);
  await new Promise((resolve) => globalThis.setTimeout(resolve, 450));
  assert.deepEqual(
    saved.map((snapshot) => snapshot.repo),
    ["latest"],
  );
});

test("flushing waits for edits made while an earlier save is pending", async () => {
  const settings: Settings = {
    repo: "first",
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
  let releaseFirst = (): void => undefined;
  let releaseSecond = (): void => undefined;
  const first = new Promise<void>((resolve) => (releaseFirst = resolve));
  const second = new Promise<void>((resolve) => (releaseSecond = resolve));
  const saved: string[] = [];
  const saver = savingSettings({
    settings: () => settings,
    setError: () => undefined,
    save: async (snapshot) => {
      saved.push(snapshot.repo);
      await (saved.length === 1 ? first : second);
    },
  });

  saver.allowWrites();
  saver.schedule();
  const flushing = saver.flush();
  let settled = false;
  void flushing.then(() => (settled = true));
  settings.repo = "second";
  saver.schedule();
  await new Promise((resolve) => globalThis.setTimeout(resolve, 450));
  releaseFirst();
  for (let attempts = 0; saved.length < 2 && attempts < 100; attempts += 1) {
    await new Promise((resolve) => globalThis.setTimeout(resolve, 5));
  }
  assert.equal(saved.length, 2, "the newer snapshot never started saving");
  const settledBeforeLatestSave = settled;
  releaseSecond();
  assert.equal(await flushing, true);
  assert.equal(settledBeforeLatestSave, false);
  assert.deepEqual(saved, ["first", "second"]);
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
  let savingStatus = -1;
  let disableQuit = -1;
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
      if (
        ts.isPropertyAccessExpression(callee) &&
        callee.name.text === "setStatus" &&
        node.arguments[0] !== undefined &&
        ts.isStringLiteral(node.arguments[0]) &&
        node.arguments[0].text === "Saving settings before quitting…" &&
        node.arguments[1] !== undefined &&
        ts.isStringLiteral(node.arguments[1]) &&
        node.arguments[1].text === "running"
      ) {
        savingStatus = node.getStart(source);
      }
    }
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      node.right.kind === ts.SyntaxKind.TrueKeyword &&
      ts.isPropertyAccessExpression(node.left) &&
      node.left.getText(source) === "ui.quit.disabled"
    ) {
      disableQuit = node.getStart(source);
    }
    node.forEachChild(inspect);
  };
  inspect(requestQuit);
  assert.ok(flush >= 0, "requestQuit never flushes settings");
  assert.ok(quit >= 0, "requestQuit no longer reaches the quit command");
  assert.ok(savingStatus >= 0, "Quit gives no feedback while settings save");
  assert.ok(disableQuit >= 0, "Quit stays active while settings save");
  assert.ok(savingStatus < flush, "Quit waits before showing its saving state");
  assert.ok(
    disableQuit < flush,
    "Quit is disabled only after its save finishes",
  );
  assert.ok(flush < quit, "requestQuit can exit before its settings flush");
});

/// A failed load must not let the next edit overwrite the file it failed on.
///
/// `suppressPersistence` only covers boot's own renders. Every later user edit
/// reaches `refresh()`, schedules a save, and atomically replaces the unreadable
/// but potentially recoverable settings file with defaults plus that edit — and
/// the load warning is not shown until provider discovery finishes, so the
/// overwrite can happen before any warning appears.
test("writes stay blocked when saved settings failed to load", async () => {
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
  const saved: string[] = [];
  const errors: string[] = [];
  const saver = savingSettings({
    settings: () => settings,
    setError: (message) => errors.push(message),
    save: async (snapshot) => {
      saved.push(snapshot.repo);
    },
  });

  saver.blockWrites("saved settings are invalid");
  settings.repo = "changed";
  saver.schedule();
  assert.equal(await saver.flush(), false);
  assert.deepEqual(saved, [], "the unreadable settings file was overwritten");
  assert.match(errors.at(-1) ?? "", /saved settings are invalid/);

  saver.allowWrites();
  assert.equal(await saver.flush(), true);
  assert.deepEqual(saved, ["changed"], "the edit was lost as well as blocked");
});
