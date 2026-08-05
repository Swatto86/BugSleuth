/**
 * Reading the frontend's own syntax tree, instead of guessing at it.
 *
 * **Six of the eleven defects this project introduced in a single day were a
 * regex over source code that matched less than existed.** Not one of them
 * failed: each returned a smaller answer, and every assertion downstream was
 * satisfied by it.
 *
 *   - Statement extraction stopped at the first line ending in `;`, which
 *     inside a `.then` callback is above the `.catch`, and reported a handled
 *     call as unhandled.
 *   - A pattern anchored on a bare name matched an import before the
 *     declaration, read the wrong block, produced an empty list — and an empty
 *     list satisfies every comparison, so a renamed lane passed.
 *   - One pattern per rustfmt layout read two enum variants of three.
 *   - Matching to the first `;` read one arm of a TypeScript union of three,
 *     because object types separate their properties with semicolons too.
 *
 * Every one of those is a question about syntax being answered by looking at
 * characters. TypeScript is already a dependency of this project — it is what
 * `npm run build` type-checks with — so its parser is right there, and it
 * cannot stop early, cannot match the wrong block, and cannot mistake a comment
 * for code. This module is the ladder's fifth rung: an installed dependency
 * already solves it.
 *
 * The helpers here are deliberately small and each is tested below against a
 * snippet whose right answer is written out by hand, because a helper that
 * quietly returns nothing is exactly what this is replacing.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));

/** Parse one file into a syntax tree. */
export function parse(fileName: string, text: string): ts.SourceFile {
  return ts.createSourceFile(fileName, text, ts.ScriptTarget.ESNext, true);
}

/** Every frontend module that ships, parsed. Test files are not shipped. */
export function frontendFiles(): ts.SourceFile[] {
  return fs
    .readdirSync(here)
    .filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts"))
    .map((name) => parse(name, fs.readFileSync(path.join(here, name), "utf8")));
}

/** Walk every node in a tree, parents included. */
export function walk(node: ts.Node, visit: (node: ts.Node) => void): void {
  visit(node);
  node.forEachChild((child) => walk(child, visit));
}

/** Every call to a named function, wherever it appears. */
export function callsTo(
  source: ts.SourceFile,
  name: string,
): ts.CallExpression[] {
  const found: ts.CallExpression[] = [];
  walk(source, (node) => {
    if (!ts.isCallExpression(node)) return;
    const callee = node.expression;
    if (ts.isIdentifier(callee) && callee.text === name) found.push(node);
  });
  return found;
}

/** The literal text of a call's nth argument, if it is a plain string. */
export function stringArgument(
  call: ts.CallExpression,
  index = 0,
): string | undefined {
  const argument = call.arguments[index];
  if (!argument) return undefined;
  if (
    ts.isStringLiteral(argument) ||
    ts.isNoSubstitutionTemplateLiteral(argument)
  ) {
    return argument.text;
  }
  return undefined;
}

/** The 1-based line a node starts on, for a message someone can act on. */
export function lineOf(source: ts.SourceFile, node: ts.Node): number {
  return source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
}

/**
 * Whether a promise-returning call's rejection is handled where it is made.
 *
 * The question the regex version kept getting wrong, in both directions. Two
 * shapes count, and nothing else does:
 *
 *   - the call sits in a chain ending in `.catch(...)`, or `.then(ok, err)`;
 *   - the call is awaited inside a `try` **belonging to the same function**.
 *
 * That last qualifier is the whole point. Awaiting does not handle anything by
 * itself — it hands the rejection to whoever holds the promise. An `async`
 * callback passed to `addEventListener` has no such holder: the promise is
 * dropped on the floor, and the rejection reaches nobody. That is a real defect
 * this project shipped on the folder-picker button, and the regex version
 * exempted it because the word `await` was present.
 */
export function rejectionIsHandled(call: ts.CallExpression): boolean {
  let node: ts.Node = call;
  let awaited = false;

  while (node.parent) {
    const parent: ts.Node = node.parent;

    // `x.catch(...)` / `x.then(ok, err)` applied to the chain we are inside.
    if (ts.isPropertyAccessExpression(parent) && parent.expression === node) {
      const method = parent.name.text;
      const outer = parent.parent;
      if (ts.isCallExpression(outer) && outer.expression === parent) {
        if (method === "catch") return true;
        if (method === "then" && outer.arguments.length >= 2) return true;
      }
    }

    if (ts.isAwaitExpression(parent)) awaited = true;

    // A `try` catches an awaited call only if the call is in its *try block*.
    //
    // The first version compared positions with a lower bound only, so a call
    // inside the `catch` or the `finally` of that same statement — which is
    // outside its protection, not inside it — was read as handled. Latent
    // rather than exploited: no shipped call sits there today. But this
    // function is the whole enforcement of "no call into Rust can fail without
    // the user being told", and the folder-picker defect it was written for was
    // exactly a call the rule wrongly believed was covered.
    if (ts.isTryStatement(parent) && node === parent.tryBlock && awaited) {
      return true;
    }

    // Crossing a function boundary ends the question: anything outside holds a
    // different promise, or none at all.
    if (ts.isFunctionLike(parent)) return false;

    node = parent;
  }
  return false;
}

/** The property names of an `interface`, or undefined if there is no such one. */
export function interfaceFields(
  sources: ts.SourceFile[],
  name: string,
): string[] | undefined {
  for (const source of sources) {
    let fields: string[] | undefined;
    walk(source, (node) => {
      if (!ts.isInterfaceDeclaration(node) || node.name.text !== name) return;
      fields = node.members
        .filter(ts.isPropertySignature)
        .map((member) => (ts.isIdentifier(member.name) ? member.name.text : ""))
        .filter(Boolean)
        .sort();
    });
    if (fields) return fields;
  }
  return undefined;
}

/**
 * A discriminated union's arms, as `kind` to its other property names.
 *
 * The shape `RunEvent` has: each arm is an object type with a literal `kind`.
 */
export function unionArms(
  sources: ts.SourceFile[],
  name: string,
  discriminant = "kind",
): Map<string, string[]> | undefined {
  for (const source of sources) {
    let arms: Map<string, string[]> | undefined;
    walk(source, (node) => {
      if (!ts.isTypeAliasDeclaration(node) || node.name.text !== name) return;
      if (!ts.isUnionTypeNode(node.type)) return;
      arms = new Map();
      for (const member of node.type.types) {
        if (!ts.isTypeLiteralNode(member)) continue;
        const properties = member.members.filter(ts.isPropertySignature);
        const tag = properties.find(
          (p) => ts.isIdentifier(p.name) && p.name.text === discriminant,
        );
        const literal = tag?.type;
        if (
          !literal ||
          !ts.isLiteralTypeNode(literal) ||
          !ts.isStringLiteral(literal.literal)
        ) {
          continue;
        }
        arms.set(
          literal.literal.text,
          properties
            .map((p) => (ts.isIdentifier(p.name) ? p.name.text : ""))
            .filter((text) => text && text !== discriminant)
            .sort(),
        );
      }
    });
    if (arms) return arms;
  }
  return undefined;
}

// ── The helpers, checked against snippets whose answer is written by hand ────

const SAMPLE = `
import { invoke } from "@tauri-apps/api/core";

export type Thing =
  | { kind: "one"; index: number; total: number }
  | { kind: "two"; model: string };

export interface Shape {
  alpha: string;
  beta: number;
}

function chained(): void {
  void invoke("a").then((x) => {
    if (!x) return;
    use(x);
  }).catch((e: unknown) => report(e));
}

function listener(): void {
  button.addEventListener("click", async () => {
    const picked = await invoke("b");
    use(picked);
  });
}

async function guarded(): Promise<void> {
  try {
    await invoke("c");
  } catch {
    report("no");
  }
}

function bare(): void {
  void invoke("d");
}
`;

test("a chain ending in .catch is handled, however many statements are inside it", () => {
  // The regex version stopped at the first `;`, which here is the `return;`
  // inside the `.then` callback — long before the `.catch`.
  const source = parse("sample.ts", SAMPLE);
  const call = callsTo(source, "invoke").find((c) => stringArgument(c) === "a");
  assert.ok(call, "did not find the chained call");
  assert.equal(rejectionIsHandled(call), true);
});

test("an awaited call in a discarded callback is not handled", () => {
  // addEventListener throws the promise away, so nothing holds the rejection.
  // The regex version exempted this because the word `await` was present, and
  // it is the defect the folder-picker button actually shipped with.
  const source = parse("sample.ts", SAMPLE);
  const call = callsTo(source, "invoke").find((c) => stringArgument(c) === "b");
  assert.ok(call, "did not find the listener call");
  assert.equal(rejectionIsHandled(call), false);
});

test("an awaited call in a catch or finally is not covered by that try", () => {
  // The hole the audit found. A `try` protects its try block; a call sitting in
  // the `catch` or the `finally` of the same statement is outside that
  // protection, and reading it as handled would let exactly the folder-picker
  // defect through again — a call the rule wrongly believed was covered.
  const source = parse(
    "s.ts",
    `
async function handler(): Promise<void> {
  try {
    thing();
  } catch {
    await invoke("in-catch");
  } finally {
    await invoke("in-finally");
  }
  await invoke("after");
}
`,
  );
  for (const command of ["in-catch", "in-finally", "after"]) {
    const call = callsTo(source, "invoke").find(
      (c) => stringArgument(c) === command,
    );
    assert.ok(call, `did not find the ${command} call`);
    assert.equal(
      rejectionIsHandled(call),
      false,
      `a call in "${command}" was read as protected by a try that does not cover it`,
    );
  }
});

test("an awaited call inside its own try is handled", () => {
  const source = parse("sample.ts", SAMPLE);
  const call = callsTo(source, "invoke").find((c) => stringArgument(c) === "c");
  assert.ok(call, "did not find the guarded call");
  assert.equal(rejectionIsHandled(call), true);
});

test("a bare voided call is not handled", () => {
  const source = parse("sample.ts", SAMPLE);
  const call = callsTo(source, "invoke").find((c) => stringArgument(c) === "d");
  assert.ok(call, "did not find the bare call");
  assert.equal(rejectionIsHandled(call), false);
});

test("every arm of a union is read, not just the first", () => {
  // Matching to the first semicolon read one arm of three, because a
  // TypeScript object type separates its properties with semicolons too.
  const arms = unionArms([parse("sample.ts", SAMPLE)], "Thing");
  assert.ok(arms, "found no union");
  assert.deepEqual([...arms.keys()].sort(), ["one", "two"]);
  assert.deepEqual(arms.get("one"), ["index", "total"]);
  assert.deepEqual(arms.get("two"), ["model"]);
});

test("interface fields come back without comments or nesting confusing them", () => {
  const fields = interfaceFields([parse("sample.ts", SAMPLE)], "Shape");
  assert.deepEqual(fields, ["alpha", "beta"]);
});

test("a name that does not exist is undefined rather than empty", () => {
  // The distinction the empty-set defect turned on. "No such type" and "a type
  // with no members" must not be the same answer, or a scan that finds nothing
  // reads as a thing that has nothing.
  assert.equal(unionArms([parse("s.ts", SAMPLE)], "Absent"), undefined);
  assert.equal(interfaceFields([parse("s.ts", SAMPLE)], "Absent"), undefined);
});

test("the real frontend parses, so the checks built on this are not vacuous", () => {
  const files = frontendFiles();
  assert.ok(files.length >= 8, `parsed only ${files.length} frontend modules`);
  const calls = files.flatMap((f) => callsTo(f, "invoke"));
  assert.ok(calls.length >= 5, `found only ${calls.length} invoke calls`);
});
