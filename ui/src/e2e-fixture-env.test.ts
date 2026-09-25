/** Linux provider children drop APPDATA; the fixture shim has to put it back. */

import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { prepareWorkspace } from "../../e2e/workspace.ts";

const root = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
);

test("unix provider shims restore APPDATA after the child allowlist strips it", () => {
  const saved = { ...process.env };
  let evidence = "";
  try {
    prepareWorkspace(root);
    evidence = process.env["BUGSLEUTH_E2E_ROOT"]!;
    const windows = process.platform === "win32";
    const child: NodeJS.ProcessEnv = {
      PATH: process.env["PATH"],
      HOME: process.env["HOME"] ?? process.env["USERPROFILE"],
      OPENCODE_CONFIG_CONTENT: JSON.stringify({
        agent: { reviewer: { permission: { edit: "deny" } } },
      }),
    };
    // Windows keeps APPDATA in the provider allowlist. Unix strips it, so the
    // shim is the only way this child can see the evidence directory.
    if (windows) {
      child["APPDATA"] = process.env["APPDATA"];
      child["SYSTEMROOT"] = process.env["SYSTEMROOT"];
      child["COMSPEC"] = process.env["COMSPEC"];
      child["PATHEXT"] = process.env["PATHEXT"];
    }
    const ran = spawnSync(
      path.join(evidence, "bin", windows ? "opencode.cmd" : "opencode"),
      ["run"],
      {
        cwd: process.env["BUGSLEUTH_E2E_REPO"],
        input: "Reply with exactly OK and nothing else.",
        encoding: "utf8",
        timeout: 15_000,
        shell: windows,
        env: child,
      },
    );
    assert.equal(ran.status, 0, ran.stderr || ran.stdout);
    assert.match(
      fs.readFileSync(path.join(evidence, "reviews.jsonl"), "utf8"),
      /seeded-repo/,
    );
  } finally {
    for (const key of Object.keys(process.env)) {
      if (!(key in saved)) delete process.env[key];
    }
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
    if (evidence) fs.rmSync(evidence, { recursive: true, force: true });
  }
});
