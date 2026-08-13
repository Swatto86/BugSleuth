/** Exact agreement between published release names and the download table. */

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
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");
const workflow = () => read(".github", "workflows", "release.yml");

function suffixes(): string[] {
  const found = [...workflow().matchAll(/"suffix":"([^"]+)"/g)].map(
    (match) => match[1]!,
  );
  assert.ok(found.length >= 3, "the release matrix suffix scan is empty");
  return found;
}

function downloadTable(): string {
  const found = /\|\s*You want\s*\|\s*Download\s*\|([\s\S]*?)\n\n/.exec(
    read("README.md"),
  );
  assert.ok(found, "no download table in the README");
  return found[0];
}

function expandedNames(table: string, prefix: string): string[] {
  const row = table.split("\n").find((line) => line.includes(`\`${prefix}-`));
  assert.ok(row, `the download table has no ${prefix} row`);
  return [...row.matchAll(/`([^`]+)`/g)].map((match) => {
    const name = match[1]!;
    return name.startsWith("-") ? prefix + name : name;
  });
}

test("release asset names exactly match the download table", () => {
  const destinations = [
    ...workflow().matchAll(
      /"\$\{\{ matrix\.(?:portable|cli) \}\}:([^"$]+)-\$\{\{ matrix\.suffix \}\}"/g,
    ),
  ]
    .map((match) => match[1]!)
    .sort();
  assert.deepEqual(destinations, ["BugSleuth-portable", "bugsleuth-cli"]);

  const table = downloadTable();
  const productName = JSON.parse(
    read("src-tauri", "tauri.conf.json"),
  ).productName;
  const escapedProductName = productName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  assert.match(table, new RegExp("`" + escapedProductName + "_x\\.y\\.z_"));

  for (const prefix of destinations) {
    assert.deepEqual(
      expandedNames(table, prefix),
      suffixes().map((suffix) => `${prefix}-${suffix}`),
    );
  }
});
