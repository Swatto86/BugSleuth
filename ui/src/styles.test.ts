/**
 * Every class the interface asks for has to exist in the stylesheet.
 *
 * Written because of a defect this project shipped and did not notice. The
 * confirmation dialog was added complete: an overlay, a focus trap, Escape to
 * dismiss. The CSS for it was written in the same change and never reached
 * `app.css` — the edit that was supposed to append it aborted partway. Nothing
 * failed. Types passed, tests passed, the build passed, because a class name
 * that matches no rule is not an error in CSS, it is just nothing.
 *
 * The result was a dialog with no overlay, no backdrop, and no stacking, drawn
 * inline in the page. It was found days later by BugSleuth's own interface lane
 * reading the code, which is a slow and expensive way to learn that a string
 * has no matching rule.
 *
 * Both directions are checked, with opposite biases, because a wrong answer
 * costs something different each way.
 *
 * Element ids are here for the same reason. `el("prove-top")` throws when the
 * markup no longer has that id, which is at least loud — but it is loud at boot
 * on a user's machine, after packaging, and the whole window fails to start.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));
const ui = path.dirname(here);

/** The frontend sources: every non-test module, plus the markup. */
function sources(): string[] {
  return [
    ...fs
      .readdirSync(here)
      .filter((n) => n.endsWith(".ts") && !n.endsWith(".test.ts"))
      .map((n) => path.join(here, n)),
    path.join(ui, "index.html"),
  ];
}

/**
 * Class names the markup and the code certainly ask for.
 *
 * Deliberately under-approximate. This set drives the "no class without a rule"
 * check, where a false positive is a failed build over a string that was never
 * a class, so only unambiguous class positions count: a `class=` or `className`
 * assignment, a `classList` call, and the second argument of the local `text()`
 * element helper. Interpolated segments are dropped; the literal words around
 * them are still real class names.
 */
function classesUsed(): Map<string, string[]> {
  const used = new Map<string, string[]>();
  const note = (name: string, where: string) => {
    if (!name) return;
    const seen = used.get(name) ?? [];
    if (!seen.includes(where)) seen.push(where);
    used.set(name, seen);
  };
  const noteList = (list: string, where: string) => {
    // A fragment that touched an interpolation is a prefix, not a class:
    // `sev sev-${severity}` names `sev` and builds `sev-critical`. Demanding a
    // rule for the literal `sev-` would be a failure over a class that does not
    // exist, which is how a check like this earns its way out of the build.
    for (const name of interpolationFreeWords(list)) note(name, where);
  };

  const assignment = /\bclass(?:Name)?\s*=\s*(?:"([^"]*)"|`([^`]*)`)/g;
  const toggled = /\bclassList\.(?:add|remove|toggle)\(\s*"([^"]*)"/g;
  // `text("span", "finding-title", …)` — the helper assigns its second argument
  // to className, so that position is as much a class as `class=` is.
  const helper = /\btext\(\s*"[a-z0-9]+"\s*,\s*(?:"([^"]*)"|`([^`]*)`)/g;

  for (const file of sources()) {
    const source = fs.readFileSync(file, "utf8");
    const where = path.basename(file);
    for (const pattern of [assignment, toggled, helper]) {
      for (const match of source.matchAll(pattern)) {
        noteList(match[1] ?? match[2] ?? "", where);
      }
    }
  }
  return used;
}

/**
 * The whole class names in a template, dropping the fragments around `${…}`.
 *
 * Splitting on whitespace alone treats `sev-` in `sev sev-${severity}` as a
 * name. It is half of one.
 */
function interpolationFreeWords(list: string): string[] {
  return list
    .split(/\$\{[^}]*\}/)
    .flatMap((part, index, parts) => {
      const words = part.split(/\s+/).filter(Boolean);
      // A part that runs into an interpolation contributes a prefix, not a
      // name — unless whitespace separates them.
      const runsInto = index < parts.length - 1 && !/\s$/.test(part);
      const runsFrom = index > 0 && !/^\s/.test(part);
      if (runsInto) words.pop();
      if (runsFrom) words.shift();
      return words;
    })
    .filter(Boolean);
}

/**
 * The prefixes that class names get built from, like `sev-`.
 *
 * A rule for `.sev-critical` has no literal user anywhere; it is reached only
 * by composition. Without these the dead-CSS check would delete four live
 * rules, which is the expensive way to be wrong.
 */
function classPrefixes(): string[] {
  const prefixes = new Set<string>();
  for (const file of sources()) {
    const source = fs.readFileSync(file, "utf8");
    for (const match of source.matchAll(/`([^`]*\$\{[^`]*)`/g)) {
      for (const part of match[1]!.split(/\$\{[^}]*\}/).slice(0, -1)) {
        const last = part.split(/\s+/).pop();
        if (last && /[\w-]$/.test(last)) prefixes.add(last);
      }
    }
  }
  return [...prefixes];
}

/**
 * Every word in a frontend string literal, so anything that could be a class.
 *
 * Deliberately over-approximate, the opposite bias to the set above. This one
 * drives the "no rule without a user" check, where a false positive means
 * deleting live CSS. A class this file cannot follow — assembled from a
 * variable, handed through a status kind, concatenated — still leaves its word
 * in some string somewhere, and a rule survives on that alone.
 */
function possibleClassWords(): Set<string> {
  const words = new Set<string>();
  for (const file of sources()) {
    const source = fs.readFileSync(file, "utf8");
    for (const match of source.matchAll(/"([^"\n]*)"|`([^`]*)`|'([^'\n]*)'/g)) {
      const body = match[1] ?? match[2] ?? match[3] ?? "";
      for (const word of body.split(/[^\w-]+/)) if (word) words.add(word);
    }
  }
  return words;
}

/** Class names any stylesheet defines a rule for. */
function classesStyled(): Set<string> {
  const styled = new Set<string>();
  for (const name of fs.readdirSync(here).filter((n) => n.endsWith(".css"))) {
    const source = fs.readFileSync(path.join(here, name), "utf8");
    // Selectors only. Declaration bodies go first, so `content: ".foo"` cannot
    // masquerade as a rule; comments go too, because they name files, and
    // `theme.test.ts` in a comment otherwise reads as a rule for `.ts`.
    const selectors = source.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/\{[^{}]*\}/g, " ");
    for (const match of selectors.matchAll(/\.([a-zA-Z][\w-]*)/g)) styled.add(match[1]!);
  }
  return styled;
}

/** Ids the markup defines. */
function idsDefined(): Set<string> {
  const html = fs.readFileSync(path.join(ui, "index.html"), "utf8");
  return new Set([...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]!));
}

/**
 * Ids the code looks up, and the prefixes it builds them from.
 *
 * `el(\`preset-${name}\`)` never spells out `preset-cheap`, so a literal-only
 * scan would miss three buttons. The prefix carries the same guarantee: nothing
 * beginning with it may vanish from the markup.
 */
function idsUsed(): { names: string[]; prefixes: string[] } {
  const names: string[] = [];
  const prefixes: string[] = [];
  for (const file of sources()) {
    const source = fs.readFileSync(file, "utf8");
    for (const match of source.matchAll(/\bel(?:<[^>]*>)?\(\s*(?:"([^"]*)"|`([^`]*)`)/g)) {
      const literal = match[1];
      if (literal !== undefined) {
        names.push(literal);
        continue;
      }
      const template = match[2]!;
      const before = template.split("${")[0]!;
      if (template.includes("${") && before) prefixes.push(before);
      else if (before) names.push(before);
    }
  }
  return { names, prefixes };
}

test("the scans find things at all, so a passing run means something", () => {
  // If a regex stopped matching — a formatter rewrote the quotes, the markup
  // moved — every assertion below would pass on an empty set.
  assert.ok(classesUsed().size >= 15, "suspiciously few classes used");
  assert.ok(classesStyled().size >= 15, "suspiciously few classes styled");
  assert.ok(possibleClassWords().size >= 100, "the string scan found almost nothing");
});

test("the precise scan sees each of the three ways a class is set", () => {
  // Each form below is the only way some live class reaches the DOM, so a
  // pattern that quietly stopped matching would take real coverage with it.
  const used = classesUsed();
  assert.ok(used.has("panel"), 'class="..." in the markup');
  assert.ok(used.has("hidden"), "classList.toggle");
  assert.ok(used.has("finding-title"), "the text() element helper");
  assert.ok(used.has("pill"), "the literal part of a `pill ${…}` template");
});

test("every element the code looks up exists in the markup", () => {
  const defined = idsDefined();
  const { names, prefixes } = idsUsed();
  assert.ok(names.length >= 15, "found almost no element lookups");
  assert.ok(prefixes.includes("preset-"), `no built ids found: ${prefixes.join(" ")}`);

  const missing = names.filter((id) => !defined.has(id));
  const unbuilt = prefixes.filter((p) => ![...defined].some((id) => id.startsWith(p)));

  assert.deepEqual(
    missing,
    [],
    "the code looks these up by id and the markup has no such element, so the " +
      "window throws on startup and shows a blank page instead",
  );
  assert.deepEqual(unbuilt, [], "ids are built from these prefixes and the markup has none");
});

test("no element asks for a class the stylesheet does not define", () => {
  const styled = classesStyled();
  const missing = [...classesUsed()]
    .filter(([name]) => !styled.has(name))
    .map(([name, where]) => `${name} (used in ${where.join(", ")})`);

  assert.deepEqual(
    missing,
    [],
    "these class names match no CSS rule, so those elements draw unstyled and " +
      "nothing else in the toolchain will say so. Add the rule, or drop the class",
  );
});

test("a class built from a prefix counts as used, not as dead style", () => {
  // `.sev-critical` has no literal user; findings.ts composes it from `sev-`.
  // If prefix detection breaks, the check below deletes four live rules.
  assert.ok(classPrefixes().includes("sev-"), classPrefixes().join(" "));
  assert.deepEqual(interpolationFreeWords("sev sev-${severity}"), ["sev"]);
  assert.deepEqual(interpolationFreeWords("pill ${x ? 'ok' : 'bad'}"), ["pill"]);
});

test("no CSS rule styles a class nothing uses", () => {
  const words = possibleClassWords();
  const prefixes = classPrefixes();
  const dead = [...classesStyled()].filter(
    (name) => !words.has(name) && !prefixes.some((p) => name.startsWith(p)),
  );

  assert.deepEqual(
    dead,
    [],
    "these CSS rules match nothing the interface renders. Dead style is how " +
      "the check above stops being believed — delete the rule, or use it",
  );
});

test("every control the matrix rebuilds can be found again afterwards", () => {
  // Toggling a lane rebuilds the whole table, so any control without a
  // `data-focus-key` loses keyboard focus to the top of the page. The first fix
  // covered three of six, and BugSleuth found the rest: typing a model id while
  // the vendor catalogue finished loading in the background rebuilt the table
  // mid-keystroke, on the first thing anyone does after opening the app.
  // Both files, because the row is built across both: `view.ts` assembles it
  // and `pickers.ts` builds the cells. Reading only `view.ts` found three of
  // six the moment the pickers moved out, and said so — a scan scoped to one
  // file silently shrinks when the code it watches is split.
  const source = ["view.ts", "pickers.ts"]
    .map((name) => fs.readFileSync(path.join(here, name), "utf8"))
    .join("\n");

  // Anything focusable that the row builder creates.
  const created = [...source.matchAll(/createElement\("(input|select|button)"\)/g)].length;
  assert.ok(created >= 6, `found only ${created} focusable controls in the row builder`);

  const keyed = [...source.matchAll(/dataset\["focusKey"\]/g)].length;
  assert.ok(
    keyed >= created - 1,
    `${created} focusable controls are rebuilt and only ${keyed} carry a focus key, ` +
      `so the rest drop keyboard focus to the top of the page on every rebuild`,
  );
});
