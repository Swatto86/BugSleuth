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

  const flush = request.indexOf("await deps.flushSettings()");
  const busy = request.indexOf("isRunning() || isApplying()");
  const quit = request.indexOf('await invoke("quit")');
  assert.ok(flush >= 0, "requestQuit no longer flushes settings");
  assert.ok(busy >= 0, "requestQuit never rechecks whether work started");
  assert.ok(quit >= 0, "requestQuit no longer reaches the quit command");
  assert.ok(
    flush < busy && busy < quit,
    "the busy recheck must happen after the flush and before exit",
  );
  assert.match(
    request,
    /!acknowledged/,
    "a confirmed quit would loop back into the warning forever",
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
  const releases = text.match(/clearing = false/g) ?? [];
  assert.ok(
    releases.length >= 2,
    `clearing is released on ${releases.length} path(s); it must be released ` +
      "on both the success and the failure of clear_saved",
  );
});
