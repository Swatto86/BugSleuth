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
import { execSync } from "node:child_process";
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
    //
    // The diagnostic matters as much as the assertion. An empty document here
    // almost always means the binary was built with plain `cargo build` rather
    // than `cargo tauri build`, so it points at the dev server instead of
    // embedding the frontend — and every other spec then fails with a confusing
    // "element not found" that says nothing about the real cause.
    await browser.waitUntil(
      async () => (await (await $$("#app")).length) > 0,
      {
        timeout: 30_000,
        timeoutMsg:
          "the app shell never rendered. If the page is blank, the binary is " +
          "probably a development build: run `cargo clean --release` then " +
          "`cargo tauri build`, because Tauri caches the dev/production choice " +
          "in its own build script and a plain cargo release build poisons it.",
      },
    );
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

  it("stops a run when asked, and says what it never reached", async function () {
    // The lifecycle change this release introduces, and the one thing no unit
    // test can show: that pressing Stop in the real window reaches Rust, kills
    // the CLI processes, and leaves a report that does not pretend to be
    // complete. Cancelling is only worth having if it actually stops spending.
    this.timeout(6 * 60_000);

    await $("#repo").setValue(REPO);
    // Every lane, so there is certain to be work still queued when we stop.
    const boxes = await $$("#matrix-body tr:first-child td.lane-cell input");
    for (const box of boxes) {
      if (!(await box.isSelected())) await box.click();
    }

    await expect($("#run")).toBeEnabled();
    await $("#run").click();

    // Stop is offered only while there is something to stop.
    await browser.waitUntil(async () => await $("#stop").isDisplayed(), {
      timeout: 60_000,
      timeoutMsg: "the Stop button never appeared during a run",
    });

    await $("#stop").click();
    // An in-app dialog, not a browser one: a native confirm would block the
    // webview and WebDriver would see nothing here at all.
    await expect($(".dialog")).toBeDisplayed();
    const buttons = await $$(".dialog-buttons button");
    let confirmed = false;
    for (const button of buttons) {
      if ((await button.getText()) === "Stop the review") {
        await button.click();
        confirmed = true;
        break;
      }
    }
    assert.ok(confirmed, "the stop dialog offered no way to confirm");

    await browser.waitUntil(
      async () => {
        const status = await $("#status").getText();
        return status.startsWith("Finished") || status.startsWith("Run failed");
      },
      {
        timeout: 4 * 60_000,
        interval: 2_000,
        timeoutMsg: `the run never came back after Stop; status: ${await $("#status").getText()}`,
      },
    );

    // The effect that matters: the report names what it never got to rather
    // than reading like a review that covered everything and found little.
    const output = await $("#output").getText();
    assert.ok(
      output.includes("cancelled"),
      `a cancelled run did not say so:
${output}`,
    );

    // And nothing is still burning quota in the background.
    const still = execSync('tasklist /FI "IMAGENAME eq claude.exe" /FI "IMAGENAME eq codex.exe"', {
      encoding: "utf8",
    });
    assert.ok(
      !/claude\.exe|codex\.exe/.test(still),
      `a provider CLI survived the cancel:
${still}`,
    );
  });

  it("keeps both themes working", async () => {
    // Dark and light are both first-class, and "match system" must follow the
    // OS rather than freezing at startup — so it is stored as the absence of an
    // override, not as a resolved value.
    const readTheme = () =>
      browser.execute(() => {
        const style = getComputedStyle(document.documentElement);
        return {
          attr: document.documentElement.getAttribute("data-theme"),
          bg: style.getPropertyValue("--bg").trim(),
        };
      });

    await $("#theme").selectByAttribute("value", "light");
    const light = await readTheme();
    await $("#theme").selectByAttribute("value", "dark");
    const dark = await readTheme();
    await $("#theme").selectByAttribute("value", "system");
    const system = await readTheme();

    assert.equal(light.attr, "light");
    assert.equal(dark.attr, "dark");
    assert.equal(system.attr, null, "match-system must remove the override, not resolve it");
    assert.notEqual(light.bg, dark.bg, "the two themes render the same background");
  });

  it("leaves the reviewed repository untouched", () => {
    // The promise the whole tool rests on. A review that modified its target
    // would be worse than useless.
    assert.ok(!fs.existsSync(path.join(REPO, ".bugsleuth-worktrees")));
  });
});
