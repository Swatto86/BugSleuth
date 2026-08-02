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
    // An awaited call propagates to its caller's try/catch; that is handled.
    if (/\bawait\s+invoke/.test(site.statement)) return false;
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
