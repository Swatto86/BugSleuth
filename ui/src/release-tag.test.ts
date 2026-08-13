/** The release tag check must execute and reject a mismatched version. */

import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
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
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

function bash(): string {
  if (process.platform !== "win32") return "bash";
  const git = spawnSync("git", ["--exec-path"], { encoding: "utf8" });
  assert.equal(git.status, 0, git.stderr);
  const candidate = path.resolve(
    git.stdout.trim(),
    "..",
    "..",
    "..",
    "bin",
    "bash.exe",
  );
  assert.ok(fs.existsSync(candidate), `Git Bash not found at ${candidate}`);
  return candidate;
}

test("the release tag check is enforced by its exit status", () => {
  const workflow = parseYaml(read(".github", "workflows", "release.yml")) as {
    jobs: { build: { steps: { name?: string; run?: string }[] } };
  };
  const step = workflow.jobs.build.steps.find(
    (candidate) => candidate.name === "Verify release tag matches app version",
  );
  assert.equal(step?.run, './scripts/check-release-tag.sh "$GITHUB_REF_NAME"');

  const version = /^version = "([^"]+)"$/m.exec(read("Cargo.toml"))?.[1];
  assert.ok(version, "Cargo.toml has no workspace version to test");
  const helper = path.join("scripts", "check-release-tag.sh");
  const matching = spawnSync(bash(), [helper, `v${version}`], {
    cwd: root,
    encoding: "utf8",
  });
  assert.equal(matching.status, 0, matching.stderr);

  const mismatched = spawnSync(bash(), [helper, "v999.999.999"], {
    cwd: root,
    encoding: "utf8",
  });
  assert.notEqual(mismatched.status, 0, "a mistagged release was accepted");
});
