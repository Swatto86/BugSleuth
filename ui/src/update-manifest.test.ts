import { strict as assert } from "node:assert";
import { test, type TestContext } from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const script = path.resolve("scripts/update-manifest.mjs");
function fixture(t: TestContext) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "bugsleuth-manifest-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const out = path.join(root, "output");
  fs.mkdirSync(out);
  const run = (mode: string, ...args: string[]) =>
    spawnSync(process.execPath, [script, mode, out, "1.2.3", ...args], {
      encoding: "utf8",
    });
  for (const [platform, asset] of [
    ["windows-x86_64", "BugSleuth 1.2.3.exe"],
    ["linux-x86_64", "BugSleuth_1.2.3_amd64.AppImage"],
  ]) {
    const bundle = path.join(root, asset!);
    fs.writeFileSync(bundle, "fixture bundle");
    fs.writeFileSync(bundle + ".sig", `public-${platform}-signature\n`);
    const result = run("fragment", platform!, bundle, bundle + ".sig");
    assert.equal(result.status, 0, result.stderr);
  }
  return { out, run, merge: () => run("merge", "windows-x86_64,linux-x86_64") };
}

test("release metadata combines signed Windows and Linux artifacts with encoded URLs", (t) => {
  const f = fixture(t);
  const result = f.merge();
  assert.equal(result.status, 0, result.stderr);
  const data = JSON.parse(
    fs.readFileSync(path.join(f.out, "latest.json"), "utf8"),
  );
  assert.equal(data.version, "1.2.3");
  assert.deepEqual(Object.keys(data.platforms).sort(), [
    "linux-x86_64",
    "windows-x86_64",
  ]);
  assert.equal(
    data.platforms["windows-x86_64"].signature,
    "public-windows-x86_64-signature",
  );
  assert.match(
    data.platforms["windows-x86_64"].url,
    /v1\.2\.3\/BugSleuth%201\.2\.3\.exe$/,
  );
  assert.match(data.platforms["linux-x86_64"].url, /\.AppImage$/);
});

test("release metadata refuses a missing required platform or a different version", (t) => {
  const missing = fixture(t);
  fs.unlinkSync(path.join(missing.out, "latest-linux-x86_64.json"));
  assert.notEqual(missing.merge().status, 0);
  assert.equal(fs.existsSync(path.join(missing.out, "latest.json")), false);
  const mismatch = fixture(t);
  const file = path.join(mismatch.out, "latest-linux-x86_64.json");
  const data = JSON.parse(fs.readFileSync(file, "utf8"));
  data.version = "1.2.2";
  fs.writeFileSync(file, JSON.stringify(data));
  assert.notEqual(mismatch.merge().status, 0);
  assert.equal(fs.existsSync(path.join(mismatch.out, "latest.json")), false);
});

test("release metadata refuses unsigned entries, escaped paths and foreign download URLs", (t) => {
  for (const change of [
    { signature: "" },
    { url: "https://attacker.invalid/app.exe" },
    {
      url: "https://github.com/Swatto86/BugSleuth/releases/download/v1.2.3/..%2Fsecret",
    },
    {
      url: "https://github.com/Swatto86/BugSleuth/releases/download/v1.2.3/missing.exe",
    },
  ]) {
    const f = fixture(t);
    const file = path.join(f.out, "latest-windows-x86_64.json");
    const data = JSON.parse(fs.readFileSync(file, "utf8"));
    Object.assign(data.platforms["windows-x86_64"], change);
    fs.writeFileSync(file, JSON.stringify(data));
    assert.notEqual(f.merge().status, 0, JSON.stringify(change));
    assert.equal(fs.existsSync(path.join(f.out, "latest.json")), false);
  }
});
