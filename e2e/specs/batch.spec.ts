import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import {
  MODEL,
  REPO,
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
    await $("#repo").setValue(REPO);
    await $("#additional-repos").setValue(second);
    await configureOneSweep(MODEL);
    const settingsFile = path.join(
      process.env["APPDATA"]!,
      "BugSleuth/settings.json",
    );
    await browser.waitUntil(
      () =>
        JSON.parse(fs.readFileSync(settingsFile, "utf8"))
          .additional_repos?.[0] === second,
      { timeout: 10_000 },
    );
    await browser.reloadSession();
    await browser.waitUntil(
      async () => await $("#additional-repos").isExisting(),
      { timeout: 30_000 },
    );
    assert.equal(await $("#additional-repos").getValue(), second);
    await browser.waitUntil(async () => await $("#run").isEnabled(), {
      timeout: 30_000,
    });
    await $("#run").click();
    await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
      timeout: 600_000,
    });
    await expect($("#repository-result-label")).toBeDisplayed();
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
    // Later single-repository journeys must not inherit this batch selection.
    await browser.execute(() => {
      const input = document.getElementById(
        "additional-repos",
      ) as HTMLTextAreaElement;
      input.value = "";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  });

  it("refuses an invalid batch before running providers and preserves the last report", async () => {
    await $("#additional-repos").setValue(path.join(REPO, "does-not-exist"));
    const previous = await $("#output").getText();
    await $("#run").click();
    await expect($("#status")).toHaveText(/cannot open/);
    await expect($("#repository-result-label")).toBeDisplayed();
    await $("#repository-result").selectByAttribute("value", "0");
    await $("#repository-result").selectByAttribute("value", "2");
    assert.equal(await $("#output").getText(), previous);
    await browser.execute(() => {
      const input = document.getElementById(
        "additional-repos",
      ) as HTMLTextAreaElement;
      input.value = "";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  });
  it("stops active and queued repositories without leaving providers running", async () => {
    const extra = ["cancel-a", "cancel-b", "cancel-c"].map((name) =>
      path.join(path.dirname(REPO), name),
    );
    for (const repo of extra)
      execFileSync("git", ["clone", "--", REPO, repo], { stdio: "pipe" });
    // WebKitGTK WebDriver drops newline characters in setValue; emulate a
    // multiline paste, including the input event that updates saved settings.
    await browser.execute((paths: string[]) => {
      const input = document.getElementById(
        "additional-repos",
      ) as HTMLTextAreaElement;
      input.value = paths.join("\n");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, extra);
    assert.equal(await $("#additional-repos").getValue(), extra.join("\n"));
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
    await browser.execute(() => {
      const input = document.getElementById(
        "additional-repos",
      ) as HTMLTextAreaElement;
      input.value = "";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  });
});
