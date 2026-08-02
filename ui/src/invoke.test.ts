/**
 * Every fire-and-forget call into Rust must say what happens when it fails.
 *
 * Written because of a defect this project shipped: pressing Stop disabled the
 * button, set the status to "Stopping — killing the sweeps in flight", and
 * called `void invoke("cancel_run")` with no rejection handler. If that command
 * failed the button stayed dead and the status stayed "Stopping" forever, with
 * a run still spending subscription quota behind it. The user's only signal was
 * an app that had quietly given up.
 *
 * A `void`-ed promise is exactly where that failure hides: `void` silences the
 * floating-promise lint, so nothing in the toolchain objects. This test is the
 * thing that objects.
 *
 * Two ways to satisfy it, and both are explicit:
 *
 *   1. Attach a `.catch(...)` that tells the user something.
 *   2. Mark the call `// invoke-may-fail-silently: <why>` when a failure
 *      genuinely has no user-visible consequence and nothing can be done.
 *
 * The second is not a loophole so much as the point: an opt-out that has to be
 * written down, next to the call, with a reason, is a decision rather than an
 * oversight.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));

const OPT_OUT = "invoke-may-fail-silently:";

interface Site {
  file: string;
  line: number;
  statement: string;
  precedingComments: string;
  /** Whether a `try {` sits between the call and the top of its function. */
  insideTry: boolean;
}

/**
 * Whether the call at `index` has a `try {` between it and its function's top.
 *
 * This exists because the first version of this file exempted every `await
 * invoke` on the reasoning that an awaited call propagates to its caller's
 * try/catch. That is true of a function with a caller. It is false of an
 * `async` callback handed to `addEventListener`, which throws the promise away
 * — and BugSleuth found exactly that on the folder-picker button, where a
 * rejection reached nobody and the button simply appeared broken.
 *
 * So the question is not "is it awaited" but "is anything catching it here".
 * Walk up through the enclosing lines, stopping at the line that opens the
 * function, and look for a `try` on the way.
 */
function inTryBlock(lines: string[], index: number): boolean {
  const indentOf = (line: string) => line.length - line.trimStart().length;
  const own = indentOf(lines[index]!);
  for (let i = index - 1; i >= 0; i -= 1) {
    const line = lines[i]!;
    if (!line.trim()) continue;
    if (indentOf(line) >= own) continue;
    if (/^\s*try\s*\{/.test(line)) return true;
    // The line that opens the enclosing function: anything above it is a
    // different scope, and a try out there does not catch a rejection that
    // happens after this function has already returned its promise.
    if (/(=>|\bfunction\b)[^)]*\{\s*$/.test(line)) return false;
  }
  return false;
}

/** Every `invoke(...)` call in the frontend, with enough context to judge it. */
function invokeSites(): Site[] {
  const sites: Site[] = [];
  for (const name of fs.readdirSync(here)) {
    if (!name.endsWith(".ts") || name.endsWith(".test.ts")) continue;
    const text = fs.readFileSync(path.join(here, name), "utf8");
    const lines = text.split("\n");

    lines.forEach((line, index) => {
      if (!/\binvoke[<(]/.test(line)) return;

      // The whole statement, not just the line: a `.catch` usually sits several
      // lines below the call in a formatted chain.
      //
      // Depth-tracked, because the obvious version — stop at the first line
      // ending in a semicolon — ends the statement on the first `return;`
      // inside a `.then` callback and never sees the `.catch` after it. That
      // reported a correctly-handled call as unhandled, which is the failure
      // mode that makes a check worse than none: it teaches you to distrust it.
      const statement: string[] = [];
      let depth = 0;
      for (let i = index; i < Math.min(lines.length, index + 40); i += 1) {
        const text = lines[i]!;
        statement.push(text);
        for (const ch of text) {
          if (ch === "(" || ch === "{" || ch === "[") depth += 1;
          if (ch === ")" || ch === "}" || ch === "]") depth -= 1;
        }
        if (depth <= 0 && /;\s*$/.test(text)) break;
      }

      // Comment lines immediately above, where an opt-out would be written.
      const comments: string[] = [];
      for (let i = index - 1; i >= 0; i -= 1) {
        const trimmed = lines[i]!.trim();
        if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) {
          comments.unshift(trimmed);
          continue;
        }
        break;
      }

      sites.push({
        file: name,
        line: index + 1,
        statement: statement.join("\n"),
        precedingComments: comments.join("\n"),
        insideTry: inTryBlock(lines, index),
      });
    });
  }
  return sites;
}

test("the frontend actually calls into Rust, so this test is not vacuous", () => {
  // A regex that silently stops matching would turn every assertion below into
  // a pass. Guard the guard.
  assert.ok(invokeSites().length >= 5, "found suspiciously few invoke sites");
});

test("no call into Rust can fail without the user being told", () => {
  const unhandled = invokeSites().filter((site) => {
    // An awaited call is handled only if something here actually catches it.
    // Awaiting is not itself a rejection handler — see inTryBlock.
    if (/\bawait\s+invoke/.test(site.statement) && site.insideTry) return false;
    // A returned promise is the caller's responsibility.
    if (/\breturn\s+invoke/.test(site.statement)) return false;
    if (site.statement.includes(".catch(")) return false;
    // `.then(onOk, onErr)` is a rejection handler too.
    if (/\.then\([^)]*,\s*\(/.test(site.statement)) return false;
    return !site.precedingComments.includes(OPT_OUT);
  });

  assert.deepEqual(
    unhandled.map((s) => `${s.file}:${s.line}`),
    [],
    `these call into Rust and ignore failure. Attach a .catch that tells the ` +
      `user, or write "// ${OPT_OUT} <why>" above the call if a failure ` +
      `genuinely has no consequence`,
  );
});

test("every silent-failure opt-out gives a reason", () => {
  // "invoke-may-fail-silently:" with nothing after it is a way of turning the
  // rule off, which is worse than not having the rule.
  const bare = invokeSites()
    .filter((site) => site.precedingComments.includes(OPT_OUT))
    .filter((site) => {
      const after = site.precedingComments.split(OPT_OUT)[1] ?? "";
      return after.replace(/[^a-z]/gi, "").length < 12;
    });
  assert.deepEqual(bare.map((s) => `${s.file}:${s.line}`), []);
});

test("a handled call is not reported just because its chain contains a semicolon", () => {
  // The bug this file shipped with: statement extraction stopped at the first
  // line ending in `;`, which inside a `.then` callback is long before the
  // `.catch`. persist.ts was reported as ignoring failure while handling it
  // properly. A check that cries wolf is worse than no check.
  const persist = invokeSites().filter((s) => s.file === "persist.ts");
  assert.equal(persist.length, 1, "expected exactly one invoke in persist.ts");
  assert.ok(
    persist[0]!.statement.includes(".catch("),
    "the extracted statement stopped before the rejection handler",
  );
});

test("an awaited call in an event listener is not treated as handled", () => {
  // The hole BugSleuth found in this very file. The folder-picker button was
  //     ui.browse.addEventListener("click", async () => {
  //       const picked = await invoke<string | null>("pick_directory");
  // and this test passed it, because the first version exempted every `await`.
  // addEventListener discards the promise, so there was no caller and the
  // rejection reached nobody: the button did nothing and said nothing.
  const listener = [
    'ui.browse.addEventListener("click", async () => {',
    '  const picked = await invoke<string | null>("pick_directory");',
    "});",
  ];
  assert.equal(inTryBlock(listener, 1), false, "an event listener has no caller to catch for it");

  const guarded = ["async function boot() {", "  try {", "    await invoke(\"preflight\");", "  } catch {}", "}"];
  assert.equal(inTryBlock(guarded, 2), true, "a try in the same function does catch it");
});
