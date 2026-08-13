/**
 * A long-running action must not be offered before the event that ends it is
 * being listened for.
 *
 * `run-finished` and `apply-finished` are the only things that clear `running`
 * and `applying`. If either subscription rejects — Tauri event registration
 * failing while command IPC still works — the action still starts, the backend
 * still completes it, and no callback ever arrives: Run, Apply, Clear and Update
 * stay blocked until the app is restarted. The Apply failure was reported only
 * through the shared status, which boot then overwrote with "Ready".
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

const parse = (name: string, text: string): ts.SourceFile =>
  ts.createSourceFile(name, text, ts.ScriptTarget.ESNext, true);

const walk = (node: ts.Node, visit: (node: ts.Node) => void): void => {
  visit(node);
  node.forEachChild((child) => walk(child, visit));
};

function declaration(
  source: ts.SourceFile,
  name: string,
): ts.FunctionDeclaration | undefined {
  let found: ts.FunctionDeclaration | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name)
      found = node;
  });
  return found;
}

const isReadyAssignment = (node: ts.Node): node is ts.BinaryExpression =>
  ts.isBinaryExpression(node) &&
  node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
  ts.isIdentifier(node.left) &&
  node.left.text === "completionEventsReady" &&
  node.right.kind === ts.SyntaxKind.TrueKeyword;

const listenedEvent = (
  node: ts.Node,
  event: string,
): node is ts.CallExpression =>
  ts.isCallExpression(node) &&
  ts.isIdentifier(node.expression) &&
  node.expression.text === "listen" &&
  node.arguments[0] !== undefined &&
  ts.isStringLiteral(node.arguments[0]) &&
  node.arguments[0].text === event;

test("long-running actions wait for their completion listeners", () => {
  const run = read("ui", "src", "run.ts");
  const main = read("ui", "src", "main.ts");
  const apply = read("ui", "src", "apply.ts");
  const html = read("ui", "index.html");

  const runSetup = declaration(parse("run.ts", run), "listenForRunEvents");
  assert.ok(runSetup, "listenForRunEvents() is gone");
  const runListeners = new Map<string, ts.CallExpression>();
  let runReady: ts.BinaryExpression | undefined;
  walk(runSetup, (node) => {
    for (const event of ["run-progress", "run-finished"]) {
      if (listenedEvent(node, event)) runListeners.set(event, node);
    }
    if (isReadyAssignment(node)) runReady = node;
  });
  assert.ok(runReady, "run listener readiness is never set");
  for (const event of ["run-progress", "run-finished"]) {
    const listener = runListeners.get(event);
    assert.ok(listener, `the ${event} listener is gone`);
    assert.ok(
      ts.isAwaitExpression(listener.parent),
      `the ${event} registration is not awaited`,
    );
    assert.ok(
      runReady.getStart() > listener.parent.getEnd(),
      `run events are marked ready before ${event} has registered`,
    );
  }
  assert.ok(
    main.includes("!runEventsReady() || busy"),
    "the Run gate does not require its completion listener",
  );

  const bindApply = declaration(parse("apply.ts", apply), "bindApply");
  assert.ok(bindApply, "bindApply() is gone");
  let applyListener: ts.CallExpression | undefined;
  walk(bindApply, (node) => {
    if (listenedEvent(node, "apply-finished")) applyListener = node;
  });
  assert.ok(applyListener, "the apply-finished listener is gone");
  let registration: ts.CallExpression | undefined;
  walk(bindApply, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      node.expression.name.text === "then" &&
      node.expression.expression === applyListener
    ) {
      registration = node;
    }
  });
  assert.ok(
    registration,
    "apply readiness does not consume listener registration",
  );
  const success = registration.arguments[0];
  assert.ok(
    success &&
      (ts.isArrowFunction(success) || ts.isFunctionExpression(success)),
    "apply listener registration has no success callback",
  );
  let applyReady = false;
  walk(success, (node) => {
    if (isReadyAssignment(node)) applyReady = true;
  });
  assert.ok(applyReady, "apply readiness is not set by registration success");
  // Whitespace-normalized: the formatter wraps this expression across lines.
  assert.ok(
    apply.replace(/\s+/g, " ").includes("!completionEventsReady || applying"),
    "the Apply gate does not require its completion listener",
  );

  // A persistent alert, because the shared status is overwritten by boot.
  assert.match(html, /id="apply-listener-error"[^>]*role="alert"/);
  // Disabled in shipped markup too, so the pre-JavaScript window is not a way in.
  assert.match(html, /id="run"[^>]*disabled/);
});

test("a failed tray-Quit listener leaves a persistent alert", () => {
  const actions = read("ui", "src", "actions.ts");
  const html = read("ui", "index.html");
  const listener = actions.indexOf('listen("confirm-quit"');
  const failure = actions.indexOf(".catch", listener);
  const assignment = actions.indexOf(
    "ui.quitListenerError.textContent = message",
    failure,
  );
  const reveal = actions.indexOf(
    'ui.quitListenerError.classList.remove("hidden")',
    failure,
  );

  assert.ok(listener >= 0, "the confirm-quit listener was not found");
  assert.ok(
    failure > listener,
    "the confirm-quit rejection handler was not found",
  );
  assert.ok(
    assignment > failure,
    "the listener error is not written to the alert",
  );
  assert.ok(reveal > failure, "the listener error alert is not revealed");
  assert.match(html, /id="quit-listener-error"[^>]*role="alert"/);
});

/// Readiness has to be redrawn once it changes, on every path.
///
/// Run is disabled in shipped markup and re-enabled by `renderPlanSummary`.
/// Boot's later `loadCatalogue()` renders on success but returns early on
/// failure — an ordinary, already-handled condition — so without an explicit
/// redraw the Run button stayed dead for the whole session.
test("boot redraws the Run gate as soon as its listener is ready", () => {
  const main = read("ui", "src", "main.ts");
  const listened = main.indexOf("await listenForRunEvents(");
  assert.ok(listened >= 0, "boot no longer subscribes to run events");
  const redrawn = main.indexOf("renderPlanSummary();", listened);
  const catalogue = main.indexOf("await loadCatalogue()", listened);
  assert.ok(
    redrawn >= 0,
    "nothing redraws the Run gate after it becomes ready",
  );
  assert.ok(
    catalogue < 0 || redrawn < catalogue,
    "the redraw is left to the catalogue load, which does not happen when it fails",
  );
});
