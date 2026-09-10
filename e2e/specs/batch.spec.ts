import { setRepositoryList } from "./repository-input.ts";
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import {
  MODEL,
  REPO,
  RUNS_ROOT,
  configureOneSweep,
  clickDialogButton,
  providerCliProcesses,
  treeDigest,
} from "./support";

/** Two real repositories through the webview, engine, CLI and disk handoff. */
describe("multiple repository reviews", () => {
  afterEach(async () => {
    // Drain provider work before WebDriver closes its owning app on a failure.
    // Once the app exits, ancestry alone cannot identify orphaned CLI jobs.
    if (await $("#stop").isDisplayed()) {
      await $("#stop").click();
      await clickDialogButton("Stop the review");
      await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
        timeout: 30_000,
      });
      await browser.waitUntil(() => providerCliProcesses().length === 0, {
        timeout: 15_000,
      });
    }
  });
  it("keeps reports and fix prompts bound to their own repository", async () => {
    const second = path.join(path.dirname(REPO), "second-repo");
    execFileSync("git", ["clone", "--", REPO, second], { stdio: "pipe" });
    const before = [treeDigest(REPO), treeDigest(second)];
    const secondInput = second + path.sep + ".";
    await setRepositoryList([REPO, secondInput]);
    await configureOneSweep(MODEL);
    const settingsFile = path.join(
      process.env["APPDATA"]!,
      "BugSleuth/settings.json",
    );
    await browser.waitUntil(
      () => {
        const saved = JSON.parse(fs.readFileSync(settingsFile, "utf8"));
        return (
          saved.additional_repos?.[0] === secondInput &&
          saved.models.length === 1 &&
          saved.models[0].id === MODEL &&
          JSON.stringify(saved.models[0].lanes) === '["correctness"]' &&
          saved.models[0].passes === 1 &&
          saved.triage_model === ""
        );
      },
      { timeout: 10_000 },
    );
    await browser.reloadSession();
    await browser.waitUntil(async () => await $("#repo").isExisting(), {
      timeout: 30_000,
    });
    assert.equal(await $("#repo").getValue(), [REPO, secondInput].join("\n"));
    await browser.waitUntil(async () => await $("#run").isEnabled(), {
      timeout: 30_000,
    });
    await $("#run").click();
    await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
      timeout: 600_000,
    });
    await expect($("#repository-result-label")).toBeDisplayed();
    await expect($("#scan-progress")).toHaveText(/1\/1 reviews returned/);
    await expect($("#scan-progress")).toHaveText(/second-repo/);
    await $("#scan-progress").scrollIntoView();
    await browser.saveScreenshot(
      path.join(path.dirname(REPO), "scan-progress.png"),
    );
    const options = await $$("#repository-result option");
    assert.equal(options.length, 3);
    await expect($("#output")).toHaveText(new RegExp("second-repo"));
    await expect($("#apply-panel")).not.toBeDisplayed();
    const promptPaths: string[] = [];
    for (const [index, repo] of [REPO, second].entries()) {
      await $("#repository-result").selectByAttribute(
        "value",
        String(index + 1),
      );
      await expect($("#findings")).toHaveText(
        /Bulk discount|threshold|discount/i,
      );
      const promptPath = (await $("#prompt-path").getText()).replace(
        "Also saved to ",
        "",
      );
      promptPaths.push(promptPath);
      assert.ok(fs.existsSync(promptPath));
      assert.ok(fs.readFileSync(promptPath, "utf8").includes(repo));
      await expect($("#apply-panel")).toBeDisplayed();
    }
    assert.equal(new Set(promptPaths).size, 2);
    assert.deepEqual([treeDigest(REPO), treeDigest(second)], before);
    await browser.reloadSession();
    await browser.waitUntil(
      async () =>
        (await $("#status").getText()).startsWith("Saved reports restored"),
      { timeout: 30_000 },
    );
    assert.equal((await $$("#repository-result option")).length, 3);
    await $("#repository-result").selectByAttribute("value", "2");
    await expect($("#findings")).toHaveText(
      /Bulk discount|threshold|discount/i,
    );
    // Later single-repository journeys must not inherit this batch selection.
    await setRepositoryList([REPO]);
  });

  it("refuses an invalid batch before running providers and preserves the last report", async () => {
    await setRepositoryList([REPO, path.join(REPO, "does-not-exist")]);
    const previous = await $("#output").getText();
    await $("#run").click();
    await expect($("#status")).toHaveText(/cannot open/);
    await expect($("#repository-result-label")).toBeDisplayed();
    await $("#repository-result").selectByAttribute("value", "0");
    await $("#repository-result").selectByAttribute("value", "2");
    assert.equal(await $("#output").getText(), previous);
    await setRepositoryList([REPO]);
  });
  it("stops active and queued repositories without leaving providers running", async () => {
    const extra = ["cancel-a", "cancel-b", "cancel-c"].map((name) =>
      path.join(path.dirname(REPO), name),
    );
    for (const repo of extra)
      execFileSync("git", ["clone", "--", REPO, repo], { stdio: "pipe" });
    // WebKitGTK WebDriver drops newline characters in setValue; emulate a
    // multiline paste, including the input event that updates saved settings.
    await setRepositoryList([REPO, ...extra]);
    assert.equal(await $("#repo").getValue(), [REPO, ...extra].join("\n"));
    await $("#run").click();
    await browser.waitUntil(
      async () => (await $("#status").getText()).startsWith("Running"),
      { timeout: 120_000 },
    );
    await $("#stop").click();
    await clickDialogButton("Stop the review");
    await expect($("#status")).toHaveText("Review stopped");
    assert.equal((await $$("#repository-result option")).length, 5);
    await $("#repository-result").selectByAttribute("value", "4");
    await expect($("#output")).toHaveText(/stopped|cancelled/i);
    await browser.waitUntil(() => providerCliProcesses().length === 0, {
      timeout: 15_000,
    });
    await setRepositoryList([REPO]);
  });

  it("clears the saved sweeps of every listed repository", async () => {
    // Both run directories were written by the batch review above. The
    // button once cleared only the first line of the list, so the second
    // repository's next review silently reused sweeps the user had been told
    // were gone.
    const second = path.join(path.dirname(REPO), "second-repo");
    await setRepositoryList([REPO, second]);
    const stored = (): string[] =>
      fs
        .readdirSync(RUNS_ROOT)
        .filter(
          (name) =>
            name.startsWith("seeded-repo") || name.startsWith("second-repo"),
        );
    const before = stored();
    assert.ok(
      before.some((name) => name.startsWith("seeded-repo")) &&
        before.some((name) => name.startsWith("second-repo")),
      `expected run directories for both repositories under ${RUNS_ROOT}: ${before.join(", ")}`,
    );

    await $("#clear-saved").click();
    await expect($(".dialog-overlay .dialog")).toBeDisplayed();
    // The dialog names every folder about to lose its sweeps, not just one.
    await expect($(".dialog-overlay .dialog")).toHaveText(/second-repo/);
    await clickDialogButton("Delete them");
    await browser.waitUntil(
      async () =>
        /^Deleted \d+ saved files across these 2 repositories/.test(
          await $("#status").getText(),
        ),
      {
        timeout: 15_000,
        timeoutMsg: `the clear did not report both repositories: ${await $("#status").getText()}`,
      },
    );
    assert.deepEqual(stored(), [], "run directories survived the clear");
    await setRepositoryList([REPO]);
  });
});
