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
import path from "node:path";

import {
  MODEL,
  REPO,
  assertSweepWritten,
  clickDialogButton,
  configureOneSweep,
  logWebviewDiagnostics,
  providerCliProcesses,
  runsDir,
  setLaneSelections,
} from "./support";

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
    await logWebviewDiagnostics();

    await browser.waitUntil(async () => (await (await $$("#app")).length) > 0, {
      timeout: 30_000,
      timeoutMsg:
        "the app shell never rendered. If the page is blank, the binary is " +
        "probably a development build: run `cargo clean --release` then " +
        "`cargo tauri build`, because Tauri caches the dev/production choice " +
        "in its own build script and a plain cargo release build poisons it.",
    });
    await expect($("h1")).toHaveText("BugSleuth");
    await expect($("#run")).toBeExisting();
    // The two controls that reach a command which writes or deletes. Neither is
    // exercised further here — one edits the repository and the other throws
    // away paid-for sweeps — but their absence from the packaged window would
    // mean a command reachable from nowhere.
    await expect($("#clear-saved")).toBeExisting();
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
    await setLaneSelections("td.lane-cell input[type=checkbox]", []);
    await expect($("#uncovered-warning")).toBeDisplayed();
    await expect($("#uncovered-warning")).toHaveText(/NOT SWEPT/);
    await expect($("#run")).toBeDisabled();
  });

  it("confirms before removing a configured model row", async () => {
    // A row holds configuration with no undo, so removal needs confirmation.
    const UNIQUE = "vendor/remove-me-12345";
    const rowCount = async () => (await $$("#matrix-body tr")).length;
    const before = await rowCount();

    await $("#add-model").click();
    await browser.waitUntil(async () => (await rowCount()) === before + 1, {
      timeout: 5_000,
      timeoutMsg: "adding a model did not add a row",
    });

    const modelInput = () => $("#matrix-body tr:last-child td.model-id input");
    await modelInput().setValue(UNIQUE);
    assert.equal(await modelInput().getValue(), UNIQUE);

    const removeButton = () => $("#matrix-body tr:last-child button");
    const actions = await $$(
      "#matrix-body tr:last-child td.lane-cell input, #matrix-body tr:last-child button",
    );
    assert.equal(actions.length, 6, "the row's labelled actions changed");
    for (const action of actions) {
      const label = (await action.getAttribute("aria-label")) ?? "";
      assert.match(label, new RegExp(`${UNIQUE}.*row ${before + 1}`));
    }

    // Removal raises the guard rather than deleting on the spot.
    await removeButton().click();
    await expect($(".dialog")).toBeDisplayed();
    assert.equal(
      await rowCount(),
      before + 1,
      "the row went before the guard was answered",
    );

    // Cancelling keeps the row exactly as it was.
    await clickDialogButton("Cancel");
    await expect($(".dialog")).not.toBeExisting();
    assert.equal(
      await rowCount(),
      before + 1,
      "cancelling still removed the row",
    );
    assert.equal(
      await modelInput().getValue(),
      UNIQUE,
      "cancelling changed the row",
    );

    // Confirming removes it.
    await removeButton().click();
    await expect($(".dialog")).toBeDisplayed();
    await clickDialogButton("Remove model");
    await browser.waitUntil(async () => (await rowCount()) === before, {
      timeout: 5_000,
      timeoutMsg: "confirming Remove did not remove the row",
    });
  });

  it("runs a real review and writes the result to disk", async function () {
    // A real model on a real repository. Slow by nature.
    this.timeout(15 * 60_000);
    const existing = runsDir();
    if (existing !== undefined)
      fs.rmSync(existing, { recursive: true, force: true });

    await $("#repo").setValue(REPO);

    // One model, one lane: the cheapest configuration that still does real work.
    await configureOneSweep(MODEL);

    await expect($("#run")).toBeEnabled();
    await $("#run").click();

    await browser.waitUntil(
      async () => (await $("#status").getText()).startsWith("Finished"),
      {
        timeout: 14 * 60_000,
        interval: 2_000,
        timeoutMsg: `run never finished; last status: ${await $("#status").getText()}`,
      },
    );
    assert.equal(
      await browser.execute(() => document.activeElement?.id),
      "status",
      "run completion dropped keyboard focus to the document body",
    );

    // Effect one: the window shows a merged report naming the defects.
    const output = await $("#output").getText();
    assert.ok(
      output.includes("distinct defect"),
      `no merged report in output:\n${output}`,
    );

    // The real window offers the only route to the repository-writing command.
    await expect($("#apply-panel")).toBeDisplayed();
    await expect($("#apply-vendor")).toBeExisting();
    await expect($("#apply-model input")).toBeExisting();
    await expect($("#apply-effort select")).toBeExisting();
    // The publish opt-in. It reaches a command that pushes to a remote, so its
    // absence from the packaged window would mean the only control gating an
    // irreversible action never shipped.
    await expect($("#push-after-apply")).toBeExisting();
    // Deliberately not asserted: whether it is ticked. That comes from this
    // machine's stored settings, and a check that fails for a reason unrelated
    // to the code is a check someone switches off. Off-by-default is covered
    // where it is decided — the Rust and TypeScript defaults.
    // Deliberately not asserted: whether the button is enabled. That depends on
    // the model stored in this machine's settings, and a check that fails for a
    // reason unrelated to the code is a check someone switches off.

    assertSweepWritten();

    const cardsBefore = await $$("#findings > *");
    const cardCount = await cardsBefore.length;
    assert.ok(cardCount > 0, "the finished run rendered no finding cards");
    await $("#repo").setValue(path.join(REPO, "missing-e2e-repository"));
    await $("#run").click();
    await browser.waitUntil(
      async () => (await $("#status").getAttribute("class")).includes("error"),
      {
        timeout: 30_000,
        timeoutMsg: "the invalid repository was not refused",
      },
    );
    const cardsAfter = await $$("#findings > *");
    assert.equal(
      await cardsAfter.length,
      cardCount,
      "a refused run erased the previous findings",
    );
    await expect($("#apply-panel")).toBeDisplayed();
  });

  it("stops a run when asked, and says what it never reached", async function () {
    // The lifecycle change this release introduces, and the one thing no unit
    // test can show: that pressing Stop in the real window reaches Rust, kills
    // the CLI processes, and leaves a report that does not pretend to be
    // complete. Cancelling is only worth having if it actually stops spending.
    this.timeout(6 * 60_000);

    await $("#repo").setValue(REPO);
    // Every lane, so there is certain to be work still queued when we stop.
    await setLaneSelections(
      "#matrix-body tr:first-child td.lane-cell input",
      [0, 1, 2, 3, 4],
    );

    await expect($("#run")).toBeEnabled();
    await $("#run").click();

    // Stop is offered only while there is something to stop.
    await browser.waitUntil(async () => await $("#stop").isDisplayed(), {
      timeout: 60_000,
      timeoutMsg: "the Stop button never appeared during a run",
    });
    await browser.waitUntil(async () => providerCliProcesses().length > 0, {
      timeout: 60_000,
      timeoutMsg: "Stop appeared but no provider CLI process ever started",
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

    // And the exact process observed above is no longer burning quota.
    await browser.waitUntil(async () => providerCliProcesses().length === 0, {
      timeout: 30_000,
      timeoutMsg: `a provider CLI survived the cancel: ${providerCliProcesses()}`,
    });
    assert.equal(providerCliProcesses(), "");
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
    assert.equal(
      system.attr,
      null,
      "match-system must remove the override, not resolve it",
    );
    assert.notEqual(
      light.bg,
      dark.bg,
      "the two themes render the same background",
    );
  });

  it("keeps keyboard focus on the matrix after removing the final row", async () => {
    // A keyboard or screen-reader user who removes the last row must not be
    // dropped to <body> and made to tab in from the top of the page again.
    const rowCount = async () => (await $$("#matrix-body tr")).length;
    while ((await rowCount()) < 2) {
      await $("#add-model").click();
    }

    const removeLastRow = async () => {
      const before = await rowCount();
      await $("#matrix-body tr:last-child button").click();
      await clickDialogButton("Remove model");
      await browser.waitUntil(async () => (await rowCount()) === before - 1, {
        timeout: 5_000,
        timeoutMsg: "the row was not removed",
      });
    };

    // Removing the focused final row lands focus on the new final row's Remove
    // button — the removal confirmation restores focus to the button it just
    // deleted, so the rebuild has to redirect it.
    await removeLastRow();
    const onLastRemove = await browser.execute(() => {
      const rows = document.querySelectorAll("#matrix-body tr");
      const last = rows[rows.length - 1];
      return (
        last != null && document.activeElement === last.querySelector("button")
      );
    });
    assert.ok(onLastRemove, "focus was lost after removing the final row");

    // Remove down to the sole remaining row, then remove it too: focus moves to
    // Add model rather than to <body>.
    while ((await rowCount()) > 0) {
      await removeLastRow();
    }
    const onAddModel = await browser.execute(
      () => document.activeElement === document.querySelector("#add-model"),
    );
    assert.ok(
      onAddModel,
      "focus did not move to Add model when the matrix emptied",
    );
  });

  it("leaves the reviewed repository untouched", () => {
    // The promise the whole tool rests on. A review that modified its target
    // would be worse than useless.
    assert.ok(
      fs.existsSync(path.join(REPO, "src", "pricing.rs")),
      `the seeded fixture is missing or empty at ${REPO} — restore it from git history`,
    );
    assert.ok(!fs.existsSync(path.join(REPO, ".bugsleuth-worktrees")));
  });
});
