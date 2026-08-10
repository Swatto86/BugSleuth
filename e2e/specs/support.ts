/**
 * Scaffolding for the acceptance journey: where the app writes, which model to
 * drive it with, how to answer its dialogs, and what to print when the webview
 * comes up blank.
 *
 * Split from `review.spec.ts` at the hard line cap. Not named `*.spec.ts` on
 * purpose — the runner globs for that suffix, and a helper module picked up as
 * a spec would open a second app session that asserts nothing.
 */

import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { splitId } from "../../ui/src/model.ts";

export const REPO = path.resolve(process.cwd(), "fixtures/seeded-repo");
export const RUNS_ROOT = path.join(
  process.env["APPDATA"] ?? os.tmpdir(),
  "BugSleuth",
  "runs",
);

/** Model used for the live run. Overridable so the journey is not pinned to one vendor. */
export const MODEL = process.env["BUGSLEUTH_E2E_MODEL"] ?? "haiku";

/** Set lane checkboxes without retaining elements invalidated by a matrix redraw. */
export async function setLaneSelections(
  selector: string,
  selectedIndexes: readonly number[],
): Promise<void> {
  const count = (await $$(selector)).length;
  for (let index = 0; index < count; index += 1) {
    const box = (await $$(selector))[index]!;
    if ((await box.isSelected()) !== selectedIndexes.includes(index)) {
      await box.click();
    }
  }
}

/** Replace arbitrary saved settings with one cheap, deterministic sweep. */
export async function configureOneSweep(modelSpec: string): Promise<void> {
  const rowCount = async () => (await $$("#matrix-body tr")).length;
  while ((await rowCount()) > 1) {
    const before = await rowCount();
    await (await $$("#matrix-body tr"))[1]!.$("button").click();
    await clickDialogButton("Remove model");
    await browser.waitUntil(async () => (await rowCount()) === before - 1, {
      timeout: 5_000,
      timeoutMsg: "removing saved model rows did not settle",
    });
  }

  const { vendor, model } = splitId(modelSpec);
  await $(
    "#matrix-body tr:first-child td:first-child select",
  ).selectByAttribute("value", vendor);
  await $("#matrix-body tr:first-child td.model-id input").setValue(model);
  await $(
    '#matrix-body tr:first-child select[aria-label="Effort for row 1"]',
  ).selectByAttribute("value", "");
  await $(
    '#matrix-body tr:first-child select[aria-label="Passes for row 1"]',
  ).selectByAttribute("value", "1");
  await setLaneSelections("#matrix-body tr:first-child td.lane-cell input", [
    0,
  ]);
  const triage = await $("#triage-severities");
  if (await triage.isSelected()) await triage.click();
}

/** Provider processes carrying BugSleuth's production-only flag combinations. */
export function providerCliProcesses(): string {
  const command = `
Get-CimInstance Win32_Process | Where-Object {
  ($_.Name -eq 'claude.exe' -and $_.CommandLine -match '--safe-mode' -and $_.CommandLine -match '--output-format') -or
  ($_.Name -eq 'codex.exe' -and $_.CommandLine -match '--ignore-rules' -and $_.CommandLine -match '--output-last-message') -or
  ($_.Name -eq 'kilo.exe' -and $_.CommandLine -match '--pure' -and $_.CommandLine -match '--agent')
} | ForEach-Object { "$($_.ProcessId) $($_.Name)" }
`;
  return execFileSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", command],
    { encoding: "utf8" },
  ).trim();
}

/** Text placed on the Windows clipboard by the real webview. */
export function clipboardText(): string {
  return execFileSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard -Raw"],
    { encoding: "utf8" },
  )
    .replace(/\r\n/g, "\n")
    .trimEnd();
}

/**
 * The app's run directory for the fixture, found by prefix.
 *
 * This used to be hardcoded as `runs/seeded-repo`, and the app now appends a
 * hash of the full repository path — two checkouts of one project share a leaf
 * name, and sharing a run directory means resume hands one of them the other's
 * sweeps. So the name is `seeded-repo-<hash>`, the hash comes from Rust's
 * DefaultHasher and is not reproducible from here, and the old constant pointed
 * at a directory that is never created. The assertion that a run had been
 * written could therefore never pass.
 */
export function runsDir(): string | undefined {
  if (!fs.existsSync(RUNS_ROOT)) return undefined;
  const match = fs
    .readdirSync(RUNS_ROOT)
    .find((name) => name === "seeded-repo" || name.startsWith("seeded-repo-"));
  return match === undefined ? undefined : path.join(RUNS_ROOT, match);
}

/** Prove the real provider wrote a useful sweep rather than only updating the UI. */
export function assertSweepWritten(): void {
  const runs = runsDir();
  assert.ok(
    runs !== undefined,
    `no seeded-repo run directory under ${RUNS_ROOT}`,
  );
  const written = fs.readdirSync(runs).filter((file) => file.endsWith(".json"));
  assert.ok(written.length > 0, `no sweep reports written to ${runs}`);

  const report = JSON.parse(
    fs.readFileSync(path.join(runs, written[0]!), "utf8"),
  );
  assert.equal(report.lane, "Correctness");
  assert.ok(
    String(report.model).includes(MODEL),
    `report model was ${report.model}`,
  );
  assert.equal(
    report.status.state,
    "swept",
    `sweep did not run: ${JSON.stringify(report.status)}`,
  );
  assert.ok(
    report.findings.length > 0,
    "the seeded fixture returned no findings, so the review did not really work",
  );
}

/** Click the in-app dialog's button whose label matches, failing loudly if none. */
export async function clickDialogButton(label: string): Promise<void> {
  const buttons = await $$(".dialog-overlay .dialog-buttons button");
  for (const button of buttons) {
    if ((await button.getText()) === label) {
      await button.click();
      return;
    }
  }
  assert.fail(`the dialog had no "${label}" button`);
}

/**
 * What the webview is *actually* showing, printed before the wait rather than
 * guessed at afterwards.
 *
 * Six runs were spent on plausible theories — a dev build, a driver mismatch, a
 * stale instance — because the failure said "blank page" and nothing about why.
 * The URL alone separates all of them: tauri://… or http://… is a production
 * binary, localhost:5173 is a development one pointed at a server that is not
 * running.
 *
 * `about:blank` with an empty document has three possible causes and they need
 * different fixes: the driver attached to a second, blank window handle; the
 * webview had not navigated yet when we looked; or the page genuinely never
 * loads. Enumerating the handles and looking again after a pause separates them.
 */
export async function logWebviewDiagnostics(): Promise<void> {
  const url = await browser
    .getUrl()
    .catch((e: unknown) => `getUrl failed: ${String(e)}`);
  const source = await browser.getPageSource().catch(() => "");
  console.log(`
[diagnostic] url=${url}`);
  console.log(`[diagnostic] page source is ${source.length} chars`);
  console.log(`[diagnostic] source head: ${source.slice(0, 300).replace(/\s+/g, " ")}
`);

  const handles = await browser.getWindowHandles().catch(() => [] as string[]);
  console.log(
    `[diagnostic] window handles: ${handles.length} ${JSON.stringify(handles)}`,
  );
  for (const handle of handles) {
    await browser.switchToWindow(handle).catch(() => undefined);
    const u = await browser.getUrl().catch(() => "?");
    const len = (await browser.getPageSource().catch(() => "")).length;
    console.log(
      `[diagnostic]   handle ${handle}: url=${u} source=${len} chars`,
    );
  }
  await new Promise((r) => setTimeout(r, 8000));
  const later = await browser.getUrl().catch(() => "?");
  const laterLen = (await browser.getPageSource().catch(() => "")).length;
  console.log(`[diagnostic] after 8s: url=${later} source=${laterLen} chars`);
}
