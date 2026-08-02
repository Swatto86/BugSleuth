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

import { frontendFiles, interfaceFields, unionArms } from "./ast.test.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const rustDir = path.join(here, "..", "..", "src-tauri", "src");

/**
 * Every `.rs` file behind the window, as one string.
 *
 * Recursive. It read only the top level, so the moment a command moved into a
 * submodule the scan stopped seeing it and reported a live command as
 * undefined — the same "matched less than existed" shape this file exists to
 * catch, in the file that catches it. Its own assertion is what found it, which
 * is the argument for these checks failing loudly rather than quietly.
 */
function rust(): string {
  const read = (dir: string): string[] =>
    fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) return read(full);
      return entry.name.endsWith(".rs") ? [fs.readFileSync(full, "utf8")] : [];
    });
  return read(rustDir).join("\n");
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

/** The field names of a `pub struct` in the Rust behind the window. */
function rustFields(name: string): string[] {
  const struct = new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`).exec(rust());
  assert.ok(struct, `no "pub struct ${name}" in src-tauri; the scan is broken`);
  // Serde renames would break the mapping silently, so refuse to guess.
  assert.ok(
    !/#\[serde\(rename/.test(struct[1]!),
    `${name} renames a field for the wire; this scan compares Rust names directly`,
  );
  const fields = [...struct[1]!.matchAll(/^\s*pub ([a-z_]+):/gm)].map((m) => m[1]!).sort();
  // Two empty lists compare equal, so a scan that stops matching turns the
  // comparison below into a pass. It happened to the lane check in this file.
  assert.ok(fields.length >= 3, `read no fields out of Rust's ${name}`);
  return fields;
}

test("the settings the window writes are the settings Rust reads", () => {
  // Settings cross this boundary as JSON, so a field added on one side alone
  // does not fail: it is simply absent, and serde fills in a default nobody
  // chose. The symptom is a control that appears to work and changes nothing.
  for (const shape of ["Settings", "ModelSetting"]) {
    const window = interfaceFields(frontendFiles(), shape);
    assert.ok(window, `no "${shape}" interface in the frontend; the scan is broken`);
    assert.deepEqual(
      window,
      rustFields(shape),
      `${shape} does not have the same fields on both sides of the boundary`,
    );
  }
});

test("a lane is spelled the same way in the window as in the engine", () => {
  // model.test.ts asserts the titles against a copy written out by hand, which
  // is the same two-places problem one level up. This reads the engine.
  const lane = fs.readFileSync(
    path.join(here, "..", "..", "crates", "bugsleuth-domain", "src", "lane.rs"),
    "utf8",
  );
  const titles = new Set([...lane.matchAll(/Lane::\w+ => "([^"]+)"/g)].map((m) => m[1]!));
  assert.ok(titles.size >= 4, "found almost no lane titles in the engine");

  // Anchored on the declaration. Matching bare `LANE_TITLES` finds the import
  // in another module first and reads the wrong block — which it did, and the
  // empty set it produced made this test pass while the lane was renamed.
  const table = /\bconst LANE_TITLES[^{]*\{([\s\S]*?)\n\}/.exec(frontend());
  assert.ok(table, "no LANE_TITLES table in the frontend; the scan is broken");
  const shown = new Set([...table[1]!.matchAll(/:\s*"([^"]+)"/g)].map((m) => m[1]!));
  assert.ok(shown.size >= 4, `read no titles out of the table: ${table[1]!}`);

  assert.deepEqual(
    [...shown].filter((t) => !titles.has(t)),
    [],
    "the window shows these lane names and the engine writes something else, " +
      "so one lane is named two ways in one product",
  );
});

test("a progress event carries the fields the window renders from", () => {
  // The window turns each event into a line of the progress log. A field
  // renamed in the engine does not fail anywhere: the property is simply
  // undefined, and the log reads "Round undefined/undefined".
  const engine = fs.readFileSync(
    path.join(here, "..", "..", "crates", "bugsleuth-engine", "src", "orchestrate.rs"),
    "utf8",
  );
  const enumeration = /pub enum RunEvent \{([\s\S]*?)\n\}/.exec(engine);
  assert.ok(enumeration, "no RunEvent enum in the engine; the scan is broken");

  // `#[serde(rename_all = "snake_case")]`, so `SweepFinished` arrives as
  // `sweep_finished` and the variant names have to be converted to compare.
  // One variant per chunk, however rustfmt chose to lay it out — a braced
  // block over several lines or all on one.
  const variants = new Map<string, string[]>();
  const body = enumeration[1]!.replace(/\/\/\/[^\n]*/g, " ");
  for (const chunk of body.split(/\n(?=\s{4}[A-Z])/)) {
    const name = /^\s*([A-Z]\w*)/.exec(chunk);
    if (!name) continue;
    const snake = name[1]!.replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase();
    variants.set(snake, [...chunk.matchAll(/(\w+):\s*\w/g)].map((f) => f[1]!).sort());
  }
  assert.ok(variants.size >= 3, `read ${variants.size} variants out of the engine`);

  // The window's side comes from TypeScript's own parser. Reading it with a
  // regex matched to the first semicolon returned one arm of three, because a
  // TypeScript object type separates its properties with semicolons too.
  const mirrored = unionArms(frontendFiles(), "RunEvent");
  assert.ok(mirrored, "no RunEvent union in the frontend; the scan is broken");

  assert.deepEqual(
    [...mirrored.keys()].sort(),
    [...variants.keys()].sort(),
    "the window and the engine disagree about which events exist",
  );
  for (const [kind, fields] of variants) {
    assert.deepEqual(mirrored.get(kind), fields, `the ${kind} event carries different fields`);
  }
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

test("the proof cap is the same number in the window, the markup and Rust", () => {
  // Three places carry this: the TypeScript constant, the input's `max`
  // attribute, and the Rust that now enforces it rather than trusting what it
  // is sent. A limit enforced only by the sender is not a limit — but three
  // copies of a number is exactly how they drift, so this compares all three.
  const model = fs.readFileSync(path.join(here, "model.ts"), "utf8");
  const inTs = /MAX_PROVE_TOP\s*=\s*(\d+)/.exec(model);
  assert.ok(inTs, "no MAX_PROVE_TOP in model.ts");

  const html = fs.readFileSync(path.join(here, "..", "index.html"), "utf8");
  const inHtml = /id="prove-top"[^>]*max="(\d+)"/.exec(html);
  assert.ok(inHtml, "the proof input has no max attribute");

  const outcome = fs.readFileSync(path.join(rustDir, "outcome.rs"), "utf8");
  const inRust = /const MAX_PROVE_TOP: usize = (\d+);/.exec(outcome);
  assert.ok(inRust, "Rust does not define a proof cap; it is trusting the window again");

  assert.equal(inHtml[1], inTs[1], "the markup and the window disagree");
  assert.equal(inRust[1], inTs[1], "Rust and the window disagree");
});
