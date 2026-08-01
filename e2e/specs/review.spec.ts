/**
 * One live acceptance journey through the real app.
 *
 * Configure a repository → run a real review with a real model → observe the
 * findings in the window *and* the run's JSON on disk → exit.
 *
 * Deliberately asserts **effects, not calls**. A finding appearing in the
 * output pane could in principle come from anywhere; a sweep report landing in
 * the app's own runs directory, naming the model that produced it, could not.
 *
 * The target is the seeded fixture — six known bugs, builds in seconds — so the
 * journey costs one cheap model invocation rather than a real repository's
 * worth of quota.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const REPO = path.resolve(process.cwd(), "fixtures/seeded-repo");
const RUNS = path.join(process.env["APPDATA"] ?? os.tmpdir(), "BugSleuth", "runs", "seeded-repo");

/** Model used for the live run. Overridable so the journey is not pinned to one vendor. */
const MODEL = process.env["BUGSLEUTH_E2E_MODEL"] ?? "haiku";

describe("BugSleuth desktop app", () => {
  it("shows its window with the shell mounted", async () => {
    // The window starts hidden and the frontend reveals it. If that ever breaks
    // the app is a process with no UI, so this is the first thing to check.
    await expect($("h1")).toHaveText("BugSleuth");
    await expect($("#run")).toBeExisting();
  });

  it("reports which provider CLIs can be started", async () => {
    await browser.waitUntil(
      async () => {
        const pills = await $$(".vendors .pill");
        // WebdriverIO's element array reports its length asynchronously.
        return (await pills.length) >= 3;
      },
      {
        timeout: 60_000,
        timeoutMsg: "provider pills never rendered",
      },
    );
    // Preflight runs real subprocesses; every configured vendor must be listed
    // whether or not it is available, because a missing one is information.
    const found = await $$(".vendors .pill");
    const names: string[] = [];
    for (const pill of found) names.push(await pill.getText());
    const joined = names.join(" ");
    for (const vendor of ["claude", "codex", "kilo"]) {
      assert.ok(joined.includes(vendor), `${vendor} missing from ${joined}`);
    }
  });

  it("warns before a run when a lane has no model", async () => {
    // The app's whole reason to exist over the command line: a lane nobody
    // covers is caught while it is still free to fix.
    for (const box of await $$("td.lane-cell input[type=checkbox]")) {
      if (await box.isSelected()) await box.click();
    }
    await expect($("#uncovered-warning")).toBeDisplayed();
    await expect($("#uncovered-warning")).toHaveText(/NOT SWEPT/);
    await expect($("#run")).toBeDisabled();
  });

  it("runs a real review and writes the result to disk", async function () {
    // A real model on a real repository. Slow by nature.
    this.timeout(15 * 60_000);
    fs.rmSync(RUNS, { recursive: true, force: true });

    await $("#repo").setValue(REPO);

    // One model, one lane: the cheapest configuration that still does real work.
    const rows = await $$("#matrix-body tr");
    const rowCount = await rows.length;
    for (let i = 1; i < rowCount; i += 1) {
      await (await $$("#matrix-body tr"))[i]!.$("button").click();
    }
    await $("#matrix-body tr:first-child td.model-id input").setValue(MODEL);
    const boxes = await $$("#matrix-body tr:first-child td.lane-cell input");
    await boxes[0]!.click(); // correctness only

    await expect($("#run")).toBeEnabled();
    await $("#run").click();

    await browser.waitUntil(async () => (await $("#status").getText()).startsWith("Finished"), {
      timeout: 14 * 60_000,
      interval: 2_000,
      timeoutMsg: `run never finished; last status: ${await $("#status").getText()}`,
    });

    // Effect one: the window shows a merged report naming the defects.
    const output = await $("#output").getText();
    assert.ok(output.includes("distinct defect"), `no merged report in output:\n${output}`);

    // Effect two — the one that cannot be faked by the frontend: the sweep's
    // own JSON is on disk, in the app's runs directory, naming the model.
    assert.ok(fs.existsSync(RUNS), `no run directory at ${RUNS}`);
    const written = fs.readdirSync(RUNS).filter((f) => f.endsWith(".json"));
    assert.ok(written.length > 0, `no sweep reports written to ${RUNS}`);

    const report = JSON.parse(fs.readFileSync(path.join(RUNS, written[0]!), "utf8"));
    assert.equal(report.lane, "Correctness");
    assert.ok(String(report.model).includes(MODEL), `report model was ${report.model}`);
    assert.equal(report.status.state, "swept", `sweep did not run: ${JSON.stringify(report.status)}`);

    // The fixture has six known defects; a sweep that found none of them means
    // the pipeline ran but did nothing useful, which a status check would miss.
    assert.ok(
      report.findings.length > 0,
      "the seeded fixture returned no findings, so the review did not really work",
    );
  });

  it("leaves the reviewed repository untouched", () => {
    // The promise the whole tool rests on. A review that modified its target
    // would be worse than useless.
    assert.ok(!fs.existsSync(path.join(REPO, ".bugsleuth-worktrees")));
  });
});
