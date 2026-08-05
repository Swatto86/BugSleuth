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
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const REPO = path.resolve(process.cwd(), "fixtures/seeded-repo");
export const RUNS_ROOT = path.join(
  process.env["APPDATA"] ?? os.tmpdir(),
  "BugSleuth",
  "runs",
);

/** Model used for the live run. Overridable so the journey is not pinned to one vendor. */
export const MODEL = process.env["BUGSLEUTH_E2E_MODEL"] ?? "haiku";

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

/** Click the in-app dialog's button whose label matches, failing loudly if none. */
export async function clickDialogButton(label: string): Promise<void> {
  const buttons = await $$(".dialog-buttons button");
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
