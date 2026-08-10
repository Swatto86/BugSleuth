/**
 * A quit while saved sweeps are being cleared must be warned about, not
 * exit mid-delete. The backend counts clearing as work that must not be killed
 * silently (tray.rs: running || applying || clearing), but the window's quit
 * guard only modelled running and applying — so a quit during a clear_saved
 * invoke, from the window's Quit or the tray's, exited part-way through
 * `remove_dir_all` and left the runs directory half-deleted.
 *
 * A real mid-delete quit needs a live window and precise timing, so this reads
 * actions.ts's own syntax instead: the guard must consult a clearing flag, and
 * that flag must be set before the invoke and cleared on both of its outcomes.
 *
 * Deliberately self-contained rather than importing the AST helpers: importing
 * another *.test.ts module re-runs its tests, which the inventory then records
 * as duplicates.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));

function actionsSource(): { source: ts.SourceFile; text: string } {
  const file = path.join(here, "actions.ts");
  const text = fs.readFileSync(file, "utf8");
  return {
    source: ts.createSourceFile(
      "actions.ts",
      text,
      ts.ScriptTarget.ESNext,
      true,
    ),
    text,
  };
}

/** The source text of a function declaration, by name (nested included). */
function functionText(source: ts.SourceFile, name: string): string | undefined {
  let found: string | undefined;
  const walk = (node: ts.Node): void => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) {
      found = node.getText();
    }
    node.forEachChild(walk);
  };
  walk(source);
  return found;
}

test("the quit guard consults the clearing state, like the tray does", () => {
  const guard = functionText(actionsSource().source, "askBeforeQuitting");
  assert.ok(guard, "askBeforeQuitting is gone from actions.ts");
  assert.match(
    guard,
    /clearing/,
    "askBeforeQuitting ignores the clearing state, so a quit during a " +
      "clear_saved delete exits with no warning",
  );
});

test("quit rechecks active work after its settings flush", () => {
  const source = actionsSource().source;
  const request = functionText(source, "requestQuit");
  assert.ok(request, "requestQuit is gone from actions.ts");

  // The complete condition, not a prefix of it. `isRunning() || isApplying()`
  // matched a recheck that had lost `clearing` and `updating()`, so a clear or
  // an update starting during the flush was exited through without a warning.
  const normalized = request.replace(/\s+/g, " ");
  const flush = normalized.indexOf("await deps.flushSettings()");
  const busy = normalized.indexOf(
    "!acknowledged && (isRunning() || isApplying() || clearing || deps.updating())",
  );
  const quit = normalized.indexOf('await invoke("quit")');
  assert.ok(flush >= 0, "requestQuit no longer flushes settings");
  assert.ok(
    busy >= 0,
    "requestQuit does not recheck all four active-work states after the flush",
  );
  assert.ok(quit >= 0, "requestQuit no longer reaches the quit command");
  assert.ok(
    flush < busy && busy < quit,
    "the busy recheck must happen after the flush and before exit",
  );

  const guard = functionText(source, "askBeforeQuitting");
  assert.ok(guard, "askBeforeQuitting is gone from actions.ts");
  assert.match(
    guard,
    /if \(yes\) requestQuit\(true\)/,
    "confirming Quit anyway does not acknowledge the active work",
  );
});

test("the clearing flag is set before the clear and released on both outcomes", () => {
  const text = actionsSource().text;
  assert.match(
    text,
    /clearing = true/,
    "nothing marks a clear as in flight, so the guard can never see it",
  );
  // One set, and a release on both the .then and the .catch: a flag cleared on
  // only one path would warn about a clear that had already finished, forever.
  //
  // Scoped to each callback. Counting `clearing = false` across the whole file
  // also counted the `let clearing = false` initializer, so deleting either
  // outcome's release still left two matches and this passing.
  const source = actionsSource().source;
  const walk = (node: ts.Node, visit: (candidate: ts.Node) => void): void => {
    visit(node);
    node.forEachChild((child) => walk(child, visit));
  };
  const mentionsClearSaved = (node: ts.Node): boolean => {
    let found = false;
    walk(node, (candidate) => {
      if (
        ts.isCallExpression(candidate) &&
        ts.isIdentifier(candidate.expression) &&
        candidate.expression.text === "invoke" &&
        candidate.arguments[0] !== undefined &&
        ts.isStringLiteral(candidate.arguments[0]) &&
        candidate.arguments[0].text === "clear_saved"
      ) {
        found = true;
      }
    });
    return found;
  };
  const releasesClearing = (node: ts.Node): boolean => {
    let found = false;
    walk(node, (candidate) => {
      if (
        ts.isBinaryExpression(candidate) &&
        candidate.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
        ts.isIdentifier(candidate.left) &&
        candidate.left.text === "clearing" &&
        candidate.right.kind === ts.SyntaxKind.FalseKeyword
      ) {
        found = true;
      }
    });
    return found;
  };
  const callbacks = new Map<string, ts.Node>();
  walk(source, (node) => {
    if (
      !ts.isCallExpression(node) ||
      !ts.isPropertyAccessExpression(node.expression)
    ) {
      return;
    }
    const name = node.expression.name.text;
    if (name !== "then" && name !== "catch") return;
    if (!mentionsClearSaved(node.expression.expression)) return;
    const callback = node.arguments[0];
    if (callback) callbacks.set(name, callback);
  });
  for (const name of ["then", "catch"]) {
    const callback = callbacks.get(name);
    assert.ok(callback, `the clear_saved .${name} callback is gone`);
    assert.ok(
      releasesClearing(callback),
      `clear_saved ${name} does not release clearing, so the app would warn ` +
        "about a delete that had already finished, forever",
    );
  }
});
