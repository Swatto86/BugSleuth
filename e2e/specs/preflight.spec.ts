import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { MODEL, REPO, configureOneSweep, clickDialogButton } from "./support";
import { setRepositoryList } from "./repository-input";

describe("repository preflight", () => {
  it("refuses a dirty batch before provider calls and preserves the paid report", async () => {
    const calls = path.join(
      process.env["BUGSLEUTH_E2E_ROOT"]!,
      "reviews.jsonl",
    );
    const second = fs.mkdtempSync(path.join(path.dirname(REPO), "preflight-"));
    execFileSync("git", ["clone", "--", REPO, second], { stdio: "pipe" });
    const git = (...args: string[]) =>
      execFileSync("git", ["-C", second, ...args], { stdio: "pipe" });
    await setRepositoryList([REPO]);
    await configureOneSweep(MODEL);
    await $("#run").click();
    await browser.waitUntil(
      async () =>
        (await $("#status").getText()).startsWith("Review incomplete"),
      { timeout: 60_000 },
    );
    const report = await $("#output").getText();
    const paid = fs.readFileSync(calls, "utf8");
    assert.ok(
      paid.length > 0,
      "the clean repository never reached the provider",
    );
    const file = path.join(second, "src/pricing.rs");
    const original = fs.readFileSync(file, "utf8");
    try {
      await setRepositoryList([REPO, second]);
      for (const state of ["tracked", "staged", "untracked"]) {
        if (state === "tracked")
          fs.appendFileSync(file, "\n// unfinished user work\n");
        if (state === "staged") git("add", "src/pricing.rs");
        if (state === "untracked") {
          git("reset", "--", "src/pricing.rs");
          fs.writeFileSync(file, original);
          fs.writeFileSync(path.join(second, "unfinished.txt"), "user work");
          git("config", "status.showUntrackedFiles", "no");
        }
        await $("#run").click();
        await browser.waitUntil(
          async () =>
            (await $("#status").getText()).includes("uncommitted changes"),
          { timeout: 15_000 },
        );
        assert.ok(
          (await $("#status").getText()).includes(second),
          "the refusal did not identify the repository",
        );
        assert.equal(
          fs.readFileSync(calls, "utf8"),
          paid,
          "repository refusal spent provider calls",
        );
        assert.equal(
          await $("#output").getText(),
          report,
          "preflight erased the paid report",
        );
        await expect($("#apply-panel")).toBeDisplayed();
        await expect($("#run")).toBeEnabled();
      }
    } finally {
      if (await $("#stop").isDisplayed()) {
        await $("#stop").click();
        await clickDialogButton("Stop the review");
        await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
          timeout: 30_000,
        });
      }
      await setRepositoryList([REPO]);
    }
  });
});
