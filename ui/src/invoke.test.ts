/**
 * Every call into Rust must say what happens when it fails.
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
 *   1. Handle the rejection — a `.catch(...)`, a `.then(ok, err)`, or an
 *      `await` inside a `try` belonging to the same function.
 *   2. Mark the call `// invoke-may-fail-silently: <why>` when a failure
 *      genuinely has no user-visible consequence and nothing can be done.
 *
 * The second is not a loophole so much as the point: an opt-out that has to be
 * written down, next to the call, with a reason, is a decision rather than an
 * oversight.
 *
 * **This ran on regular expressions and was wrong twice.** It reported a
 * correctly-handled call as unhandled, because statement extraction stopped at
 * the first line ending in a semicolon — which inside a `.then` callback is
 * above the `.catch`. And it exempted every `await invoke`, on the reasoning
 * that an awaited call propagates to its caller; an `async` callback given to
 * `addEventListener` has no caller, and the folder-picker button shipped
 * swallowing its own failure because of it. Both are questions about syntax,
 * and both are now answered by TypeScript's own parser — see `ast.test.ts`.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import {
  callsTo,
  frontendFiles,
  lineOf,
  rejectionIsHandled,
  stringArgument,
} from "./ast.test.ts";

const OPT_OUT = "invoke-may-fail-silently:";

interface Site {
  file: string;
  line: number;
  command: string;
  handled: boolean;
  optOut: string;
}

/**
 * The comments written above a call, on any statement that encloses it.
 *
 * Not just the innermost statement. `if (yes) void invoke("quit");` puts the
 * call in an expression statement of its own, while the comment explaining it
 * sits above the `if` — and a lookup that stopped at the innermost statement
 * read no comment at all and reported a documented decision as an oversight.
 * The climb stops at the function boundary, because a comment out there is
 * about something else.
 */
function commentsAbove(source: ts.SourceFile, call: ts.CallExpression): string {
  const text = source.getFullText();
  const found: string[] = [];
  let node: ts.Node = call;
  while (node.parent && !ts.isFunctionLike(node)) {
    if (ts.isStatement(node)) {
      for (const range of ts.getLeadingCommentRanges(
        text,
        node.getFullStart(),
      ) ?? []) {
        found.push(text.slice(range.pos, range.end));
      }
    }
    node = node.parent;
  }
  return found.join("\n");
}

function invokeSites(): Site[] {
  return frontendFiles().flatMap((source) =>
    callsTo(source, "invoke").map((call) => ({
      file: source.fileName,
      line: lineOf(source, call),
      command: stringArgument(call) ?? "<not a literal>",
      handled: rejectionIsHandled(call),
      optOut: commentsAbove(source, call),
    })),
  );
}

test("the frontend actually calls into Rust, so this test is not vacuous", () => {
  const sites = invokeSites();
  assert.ok(sites.length >= 5, `found only ${sites.length} invoke sites`);
  // Named commands, not just a count: a parser that returned call expressions
  // with no arguments would still satisfy a count.
  const commands = sites.map((s) => s.command);
  assert.ok(commands.includes("start_run"), commands.join(" "));
  assert.ok(commands.includes("pick_directory"), commands.join(" "));
});

test("no call into Rust can fail without the user being told", () => {
  const unhandled = invokeSites()
    .filter((site) => !site.handled && !site.optOut.includes(OPT_OUT))
    .map((site) => `${site.file}:${site.line} (${site.command})`);

  assert.deepEqual(
    unhandled,
    [],
    `these call into Rust and ignore failure. Handle the rejection, or write ` +
      `"// ${OPT_OUT} <why>" above the call if a failure genuinely has no consequence`,
  );
});

test("every silent-failure opt-out gives a reason", () => {
  // "invoke-may-fail-silently:" with nothing after it is a way of turning the
  // rule off, which is worse than not having the rule.
  const bare = invokeSites()
    .filter((site) => site.optOut.includes(OPT_OUT))
    .filter((site) => {
      const after = site.optOut.split(OPT_OUT)[1] ?? "";
      return after.replace(/[^a-z]/gi, "").length < 12;
    })
    .map((site) => `${site.file}:${site.line}`);
  assert.deepEqual(bare, []);
});

test("the two calls this rule was wrong about are classified correctly", () => {
  // Both directions of the regex version's failure, asserted against the real
  // code rather than a snippet, so a regression in either shows up here.
  const sites = invokeSites();

  const settings = sites.find((s) => s.command === "save_settings");
  assert.ok(settings, "save_settings is no longer called");
  assert.equal(
    settings.handled,
    true,
    "a chain with a .catch read as unhandled",
  );

  const picker = sites.find((s) => s.command === "pick_directory");
  assert.ok(picker, "pick_directory is no longer called");
  assert.ok(
    picker.handled || picker.optOut.includes(OPT_OUT),
    "the folder picker is swallowing its failure again",
  );
});
