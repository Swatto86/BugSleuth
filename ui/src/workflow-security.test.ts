import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { parse as parseYaml } from "yaml";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);
const workflow = fs
  .readFileSync(path.join(root, ".github", "workflows", "release.yml"), "utf8")
  .replace(/\r\n?/g, "\n");
const verifyWorkflow = fs
  .readFileSync(path.join(root, ".github", "workflows", "verify.yml"), "utf8")
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

function usesValues(value: unknown): string[] {
  if (Array.isArray(value)) return value.flatMap(usesValues);
  if (!value || typeof value !== "object") return [];
  const record = value as Record<string, unknown>;
  return [
    ...(typeof record.uses === "string" ? [record.uses] : []),
    ...Object.values(record).flatMap(usesValues),
  ];
}

function actionRef(action: string, source = workflow): string {
  const matches = usesValues(parseYaml(source)).filter((ref) => {
    const at = ref.lastIndexOf("@");
    return at > 0 && ref.slice(0, at) === action;
  });
  assert.equal(matches.length, 1, `expected one ${action} use`);
  const ref = matches[0]!;
  return ref.slice(ref.lastIndexOf("@") + 1);
}

test("action references match the complete YAML uses value", () => {
  const revision = "11d5960a326750d5838078e36cf38b85af677262";
  assert.equal(
    actionRef(
      "actions/checkout",
      `steps:\n  - uses: actions/checkout@${revision}`,
    ),
    revision,
  );
  assert.throws(() =>
    actionRef(
      "actions/checkout",
      `steps:\n  - uses: attacker/actions/checkout@${revision}`,
    ),
  );
  assert.throws(() =>
    actionRef("actions/checkout", `# uses: actions/checkout@${revision}`),
  );
});

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
  assert.match(publish, /softprops\/action-gh-release@/);
  assert.equal(workflow.match(/^[ \t]*contents: write$/gm)?.length, 1);
});

test("the release checkout is pinned to an immutable revision", () => {
  assert.match(actionRef("actions/checkout"), /^[0-9a-f]{40}$/);
});

test("the release Node setup is pinned to an immutable revision", () => {
  assert.match(actionRef("actions/setup-node"), /^[0-9a-f]{40}$/);
});

test("the release publisher is pinned to an immutable revision", () => {
  assert.match(actionRef("softprops/action-gh-release"), /^[0-9a-f]{40}$/);
});

test("checksum fallback excludes and verifies its own manifest", () => {
  const build = section("build", 2);
  const start = build.indexOf("- name: Collect and verify artifacts");
  const end = build.indexOf("- name: Upload release artifacts", start);
  assert.ok(start >= 0 && end > start, "could not isolate artifact collection");
  const collect = build.slice(start, end);

  assert.match(collect, /sums="SHA256SUMS-\$\{\{ matrix\.suffix \}\}\.txt"/);
  assert.match(collect, /rm -f "\$sums"/);
  assert.match(collect, /if command -v sha256sum >\/dev\/null 2>&1; then/);
  assert.match(collect, /sha256sum -c "\$sums"/);
  assert.match(collect, /shasum -a 256 -c "\$sums"/);
});

test("the verification checkout is pinned to an immutable revision", () => {
  assert.match(actionRef("actions/checkout", verifyWorkflow), /^[0-9a-f]{40}$/);
});
