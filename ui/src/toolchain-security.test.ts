/**
 * The native executables the E2E suite runs are verified before they run.
 *
 * `msedgedriver.exe` arrives over the network and is then executed with the
 * developer's privileges — three separate sinks, each of which was unguarded:
 * the freshly downloaded copy, the cached copy the version check executes on
 * every later setup run, and the real `tauri-driver` spawn in the E2E config.
 * A check confined to the installer protects none of the others, so each sink
 * is asserted where it is, bounded so an earlier unrelated call cannot satisfy
 * a later one.
 */

import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
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

const VERIFIER = "assert-microsoft-signature.ps1";

/** The setup script's text between two known landmarks, both required. */
function between(source: string, from: string, to: string): string {
  const start = source.indexOf(from);
  assert.ok(start >= 0, `setup-e2e.ps1 no longer contains ${from}`);
  const end = source.indexOf(to, start + from.length);
  assert.ok(
    end > start,
    `setup-e2e.ps1 no longer contains ${to} after ${from}`,
  );
  return source.slice(start, end);
}

test("the signature verifier refuses an executable Microsoft did not sign", (t) => {
  // Authenticode is a Windows trust-store facility; there is nothing to check
  // elsewhere; Authenticode itself is Windows-only.
  if (process.platform !== "win32") {
    t.skip("Authenticode verification is Windows-only");
    return;
  }
  const unsigned = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), "bugsleuth-unsigned-")),
    "probe.exe",
  );
  fs.writeFileSync(unsigned, "MZ this is not a signed binary");
  const checked = spawnSync(
    "pwsh",
    [
      "-NoProfile",
      "-NonInteractive",
      "-File",
      path.join(root, "scripts", VERIFIER),
      "-Path",
      unsigned,
    ],
    { stdio: "pipe", windowsHide: true, shell: false },
  );
  fs.rmSync(path.dirname(unsigned), { recursive: true, force: true });
  assert.notEqual(
    checked.status,
    0,
    "the verifier accepted an unsigned executable",
  );
});

test("the E2E run verifies the native driver before spawning it", () => {
  const conf = read("e2e", "wdio.conf.ts");
  // At the execution boundary, not only in the installer. Setup can be skipped
  // and the file can change afterwards, so the check that matters is the one
  // immediately ahead of the spawn that hands the path to tauri-driver.
  const spawnAt = conf.indexOf("tauriDriver = spawn(");
  assert.ok(spawnAt >= 0, "wdio.conf.ts no longer spawns tauri-driver");
  const assertAt = conf.lastIndexOf(
    "assertMicrosoftDriver(nativeDriver)",
    spawnAt,
  );
  assert.ok(
    assertAt >= 0 && assertAt < spawnAt,
    "tauri-driver is handed an unverified native driver",
  );
  assert.ok(
    conf.includes(VERIFIER),
    "the E2E runner does not use the shared signature verifier",
  );
  // A path with a space is one argv entry, not a command line to re-parse.
  assert.ok(
    /shell: false/.test(conf.slice(assertAt, spawnAt + 400)),
    "the driver path is passed through a shell",
  );
});

test("tauri-driver is installed at one reviewed version", () => {
  const setup = read("scripts", "setup-e2e.ps1");
  // `--locked` pins the crate's own dependencies, not the crate. Without a
  // top-level version a newly published compromised release is simply the
  // newest compatible one, and setup installs and runs it.
  assert.ok(
    !/cargo install tauri-driver --locked/.test(setup),
    "setup still installs whichever tauri-driver release Cargo picks",
  );
  const pin = /\$wantedTauriDriver = '([\d.]+)'/.exec(setup);
  assert.ok(pin, "setup-e2e.ps1 no longer declares a reviewed tauri-driver");
  // One variable, read by both the install and the already-installed check, so
  // the two cannot drift into installing one version and accepting another.
  assert.ok(
    setup.includes(
      'cargo install tauri-driver --version "=$wantedTauriDriver" --locked --force',
    ),
    `the install command does not pin tauri-driver ${pin[1]!}`,
  );
  assert.ok(
    setup.includes("^tauri-driver v$([regex]::Escape($wantedTauriDriver)):"),
    "the installed-version check does not use the reviewed version",
  );
});

test("a cached EdgeDriver is verified before its version is read", () => {
  const setup = read("scripts", "setup-e2e.ps1");
  // Bounded to the cached branch, ahead of the first `& $driver`. A driver
  // written by an earlier compromised download used to survive every later
  // setup run, because the version check executed it before deciding whether
  // to replace it.
  const cached = between(
    setup,
    "if (Test-Path $driver) {",
    "$driver --version",
  );
  assert.ok(
    cached.includes(VERIFIER),
    "the cached driver is executed before its publisher is verified",
  );
});

test("a downloaded EdgeDriver is verified before it is executed", () => {
  const setup = read("scripts", "setup-e2e.ps1");
  // Bounded between the extraction and the first execution of the new file, so
  // the cached branch's own verifier call cannot stand in for this one.
  const afterExtract = between(setup, "Expand-Archive", "$driver --version");
  assert.ok(
    afterExtract.includes(VERIFIER),
    "the downloaded driver is executed before its publisher is verified",
  );
});
