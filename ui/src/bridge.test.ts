/**
 * The JavaScript and the Rust have to agree about names nothing checks.
 *
 * Every call across this boundary is a string. `invoke("start_run")` names a
 * Rust function; `listen("run-progress")` names an event Rust emits. TypeScript
 * sees a string. Rust sees a string. Neither compiler sees the other side, so a
 * rename on one side and not the other builds clean and fails only when a user
 * presses the button — which, for a run that takes tens of minutes, may be a
 * long way from the change that broke it.
 *
 * This is the same defect as the missing dialog CSS in `styles.test.ts`: a name
 * with no definition, in a language where that is not an error. It found a live
 * one on the first run — `plan_run` was registered over IPC with no caller
 * anywhere, an exposed command that existed only to be forgotten.
 *
 * Checked both ways for both kinds of name. A name used with no definition is
 * a runtime failure. A definition nothing uses is the dead surface above, and
 * for a command it is dead surface reachable from the page.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));
const rustDir = path.join(here, "..", "..", "src-tauri", "src");

/** Every `.rs` file behind the window, as one string. */
function rust(): string {
  return fs
    .readdirSync(rustDir)
    .filter((n) => n.endsWith(".rs"))
    .map((n) => fs.readFileSync(path.join(rustDir, n), "utf8"))
    .join("\n");
}

/** Every non-test frontend module, as one string. */
function frontend(): string {
  return fs
    .readdirSync(here)
    .filter((n) => n.endsWith(".ts") && !n.endsWith(".test.ts"))
    .map((n) => fs.readFileSync(path.join(here, n), "utf8"))
    .join("\n");
}

function matches(source: string, pattern: RegExp): Set<string> {
  return new Set([...source.matchAll(pattern)].map((m) => m[1]!));
}

/** Commands the page calls. `invoke<T>("name")` and `invoke("name")` both. */
const invoked = () => matches(frontend(), /\binvoke(?:<[^>]*>)?\(\s*"([a-z_]+)"/g);

/** Commands Rust defines — the attribute, then the function it sits on. */
const defined = () =>
  matches(rust(), /#\[tauri::command\][\s\S]{0,80}?\bfn\s+([a-z_]+)/g);

/**
 * Commands Rust actually exposes to the page.
 *
 * Any module, not just `commands` — the catalogue registers its own, and a scan
 * that assumed a single module reported a live command as unregistered.
 */
const registered = () => {
  const handler = /generate_handler!\s*\[([\s\S]*?)\]/.exec(rust());
  assert.ok(handler, "no invoke handler found in src-tauri; the scan is broken");
  return new Set(
    handler[1]!
      .split(",")
      .map((entry) => entry.replace(/\/\/[^\n]*/g, "").trim())
      .filter((entry) => /^(?:\w+::)*[a-z_]+$/.test(entry))
      .map((entry) => entry.split("::").pop()!),
  );
};

const listened = () => matches(frontend(), /\blisten(?:<[\s\S]*?>)?\(\s*"([a-z-]+)"/g);
const emitted = () => matches(rust(), /\bemit\w*\(\s*"([a-z-]+)"/g);

test("both sides are actually being read", () => {
  // Every assertion below passes trivially on an empty set, so the scans have
  // to be shown to work before their results mean anything.
  assert.ok(invoked().size >= 5, "found almost no invoke calls");
  assert.ok(defined().size >= 5, "found almost no #[tauri::command] functions");
  assert.ok(registered().size >= 5, "found almost no registered commands");
  assert.ok(listened().size >= 1, "found no event listeners");
  assert.ok(emitted().size >= 1, "found no emitted events");
});

test("every command the page calls exists in Rust and is exposed", () => {
  const rustCommands = defined();
  const exposed = registered();
  const broken = [...invoked()].filter((name) => !rustCommands.has(name));
  const unexposed = [...invoked()].filter(
    (name) => rustCommands.has(name) && !exposed.has(name),
  );

  assert.deepEqual(broken, [], "the page calls these; no Rust function answers to the name");
  // Defined but left out of the handler list is the subtler half: the function
  // is right there in the file, so a reader checking by eye finds it.
  assert.deepEqual(
    unexposed,
    [],
    "these exist in Rust but are missing from generate_handler!, so the call " +
      "fails at runtime with the function sitting in plain sight",
  );
});

test("no command is exposed to the page that the page never calls", () => {
  const calls = invoked();
  const dead = [...registered()].filter((name) => !calls.has(name));

  assert.deepEqual(
    dead,
    [],
    "these are reachable from the page and called by nothing. That is surface " +
      "with no user: delete the command, or call it",
  );
});

test("every event has both an emitter and a listener", () => {
  const sent = emitted();
  const heard = listened();

  assert.deepEqual(
    [...heard].filter((name) => !sent.has(name)),
    [],
    "the page waits for these events and nothing in Rust ever sends them",
  );
  assert.deepEqual(
    [...sent].filter((name) => !heard.has(name)),
    [],
    "Rust sends these and nothing is listening, so whatever they report is lost",
  );
});
