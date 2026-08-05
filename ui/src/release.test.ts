/**
 * The install instructions must name files the release actually publishes.
 *
 * A download table is the first thing a new user acts on, and it is the one
 * piece of documentation where being wrong wastes their time immediately: they
 * go to the releases page, look for a name that is not there, and conclude the
 * build is broken. Nothing else in this repository would notice — the README is
 * prose, and prose does not fail to compile.
 *
 * This is the same shape as every other check here: two places that must agree,
 * with no compiler spanning them. The workflow builds the names from a matrix,
 * so the comparison is on the stable stems rather than on whole filenames, and
 * the suffixes are checked separately against the matrix itself.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

/**
 * Read a file with its line endings normalised.
 *
 * These files are checked out with CRLF on Windows, and a pattern written with
 * `\n` in it then matches nothing — silently, producing an empty result that
 * satisfies whatever it is compared against. That has already happened twice in
 * this repository, so the normalising happens once, here.
 */
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

const readme = () => read("README.md");
const workflow = () => read(".github", "workflows", "release.yml");

/**
 * The platform suffixes the release can build for.
 *
 * All of them, not just the ones a tag push publishes: tagging builds Windows
 * only, and Linux and macOS are built by running the workflow manually. They
 * are still offered, so the README still has to describe them — a platform that
 * is buildable but undocumented is one nobody knows to ask for.
 *
 * Read out of the `plan` job's JSON rather than YAML keys. When the matrix
 * moved there this pattern matched nothing, and `>= 3` is what said so instead
 * of the empty list quietly satisfying every comparison downstream.
 */
function suffixes(): string[] {
  const found = [...workflow().matchAll(/"suffix":"([^"]+)"/g)].map(
    (m) => m[1]!,
  );
  assert.ok(
    found.length >= 3,
    `the release matrix names ${found.length} platforms`,
  );
  return found;
}

test("the README's download table names assets the workflow builds", () => {
  const text = readme();
  const yaml = workflow();

  // Only the table, so a stem mentioned in passing elsewhere does not count.
  const table = /\| You want \| Download \|([\s\S]*?)\n\n/.exec(text);
  assert.ok(table, "no download table in the README; the scan is broken");

  const stems = [
    ...table[1]!.matchAll(
      /`([A-Za-z][\w.-]*?)[_-](?:x\.y\.z|windows|linux|macos)/g,
    ),
  ]
    .map((m) => m[1]!)
    .filter((stem, index, all) => all.indexOf(stem) === index);
  assert.ok(
    stems.length >= 3,
    `read ${stems.length} asset names out of the table`,
  );

  const missing = stems.filter((stem) => !yaml.includes(stem));
  assert.deepEqual(
    missing,
    [],
    "the README tells people to download these and the release workflow builds " +
      "no such thing. Someone following it lands on a releases page with no " +
      "matching file and assumes the build is broken",
  );
});

test("every platform the release builds for is offered in the README", () => {
  const text = readme();
  const absent = suffixes().filter((suffix) => !text.includes(suffix));
  assert.deepEqual(
    absent,
    [],
    "the release publishes for these platforms and the README never mentions " +
      "them, so nobody on that platform knows there is a build",
  );
});

test("the README still promises a single file that runs with nothing installed", () => {
  // The project's own release rule, and the reason the portable binary exists
  // at all. If this sentence goes, the rule has quietly been dropped.
  const text = readme().toLowerCase();
  assert.ok(
    text.includes("runs with nothing installed") ||
      text.includes("no installer"),
    "the README no longer promises a standalone binary",
  );
  assert.ok(
    workflow().includes("BugSleuth-portable-"),
    "the release workflow no longer publishes a portable binary",
  );
});

test("the documentation does not point at files that no longer exist", () => {
  // ARCHITECTURE.md named `scripts/check-file-size.ps1` for hours after it was
  // deleted, and `scripts/verify.ps1` as the gate after it became a shim. A
  // document describing a system that is not there is the same defect as a
  // comment describing behaviour the code does not implement — it is worse in
  // documentation, because a reader has no compiler to disagree with it.
  const docs = ["README.md", "ARCHITECTURE.md", "RUNBOOK.md", "PROGRESS.md"];
  const missing: string[] = [];

  for (const doc of docs) {
    const text = read(doc);
    for (const match of text.matchAll(/`(scripts\/[\w.-]+)`/g)) {
      const named = match[1]!;
      if (!fs.existsSync(path.join(root, named)))
        missing.push(`${doc}: ${named}`);
    }
  }
  // Guard the guard: these documents do name scripts, so an empty result must
  // mean they all exist rather than that the pattern stopped matching.
  const mentioned = docs.flatMap((doc) => [
    ...read(doc).matchAll(/`(scripts\/[\w.-]+)`/g),
  ]).length;
  assert.ok(
    mentioned >= 3,
    `found only ${mentioned} script references to check`,
  );

  assert.deepEqual(
    missing,
    [],
    "the documentation tells a reader to run these and they are not in the repository",
  );
});
