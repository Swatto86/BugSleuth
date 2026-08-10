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
const workflow = fs
  .readFileSync(path.join(root, ".github", "workflows", "release.yml"), "utf8")
  .replace(/\r\n?/g, "\n");

function section(name: string, indent: number): string {
  const lines = workflow.split("\n");
  const header = `${" ".repeat(indent)}${name}:`;
  const starts = lines.flatMap((line, index) =>
    line === header ? [index] : [],
  );
  assert.equal(starts.length, 1, `expected one ${name} section`);
  const start = starts[0]!;
  const sibling = new RegExp(`^ {${indent}}\\S[^:]*:\\s*$`);
  const end = lines.findIndex(
    (line, index) => index > start && sibling.test(line),
  );
  return lines.slice(start, end < 0 ? undefined : end).join("\n");
}

test("only the release publisher receives repository write authority", () => {
  const permissions = section("permissions", 0);
  const build = section("build", 2);
  const publish = section("publish", 2);

  assert.match(permissions, /^  contents: read$/m);
  assert.doesNotMatch(build, /contents: write/);
  assert.match(
    build,
    /actions\/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02/,
  );
  assert.match(publish, /^    needs: build$/m);
  assert.match(publish, /^      contents: write$/m);
  assert.match(
    publish,
    /actions\/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093/,
  );
  assert.match(publish, /softprops\/action-gh-release@v2/);
  assert.equal(workflow.match(/^[ \t]*contents: write$/gm)?.length, 1);
});
