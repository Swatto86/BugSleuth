import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { live } from "../workspace.ts";
import {
  MODEL,
  REPO,
  configureOneSweep,
  clickDialogButton,
  providerCliProcesses,
} from "./support.ts";

/** Real CLI subprocesses and git effects, using separate model selections. */
describe("parallel repository fixing", () => {
  afterEach(async () => {
    if (await $("#stop").isDisplayed()) {
      await $("#stop").click();
      await clickDialogButton(
        (await $("#stop").getText()).includes("fixes")
          ? "Stop applying"
          : "Stop the review",
      );
      await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
        timeout: 30_000,
      });
    }
    await browser.waitUntil(() => providerCliProcesses().length === 0, {
      timeout: 15_000,
    });
    const marker = path.join(
      process.env["BUGSLEUTH_E2E_ROOT"]!,
      "parallel-apply",
    );
    if (fs.existsSync(marker)) fs.unlinkSync(marker);
    await $("#repo").setValue(REPO);
    await browser.execute(() => {
      const input = document.getElementById(
        "additional-repos",
      ) as HTMLTextAreaElement;
      input.value = "";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  });

  for (const vendors of [
    ["claude", "opencode"],
    ["codex", "codex"],
  ]) {
    it(`runs ${vendors.join(" and ")} independently and retains each model and result`, async function () {
      if (live) this.skip();
      await $("#check-signin").waitForEnabled({ timeout: 60_000 });
      const root = process.env["BUGSLEUTH_E2E_ROOT"]!;
      const models =
        vendors[0] === "codex"
          ? ["fixture-first", "fixture-second"]
          : ["haiku", "local-fixture/fixer:latest"];
      const repos = [`fix-${vendors[0]}-first`, `fix-${vendors[1]}-second`].map(
        (name) => path.join(root, name),
      );
      for (const repo of repos) {
        fs.cpSync(REPO, repo, {
          recursive: true,
          filter: (file) => ![".git", "target"].includes(path.basename(file)),
        });
        const git = (...args: string[]) =>
          execFileSync("git", ["-C", repo, ...args], { stdio: "pipe" });
        git("init", "-b", "main");
        git("add", ".");
        git(
          "-c",
          "user.name=Acceptance",
          "-c",
          "user.email=acceptance@localhost",
          "commit",
          "-m",
          "Fixture",
        );
      }
      await $("#repo").setValue(repos[0]);
      await $("#additional-repos").setValue(repos[1]);
      await configureOneSweep(MODEL);
      await $("#run").click();
      await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
        timeout: 90_000,
      });
      fs.writeFileSync(path.join(root, "parallel-apply"), "");
      fs.writeFileSync(path.join(root, "applies.jsonl"), "");
      await expect($("#fix-board")).toBeDisplayed();
      for (const [index, vendor] of vendors.entries()) {
        const row = `.fix-repository:nth-child(${index + 1})`;
        await $(`${row} [data-fix-vendor]`).selectByAttribute("value", vendor);
        await $(`${row} input`).setValue(models[index]);
      }
      await browser.saveScreenshot(
        path.join(root, `assignments-${vendors[0]}.png`),
      );
      for (let index = 0; index < vendors.length; index++) {
        await $(
          `.fix-repository:nth-child(${index + 1}) [data-fix-start]`,
        ).click();
        await clickDialogButton("Apply the fixes");
      }
      await browser.waitUntil(
        () => {
          const log = path.join(root, "applies.jsonl");
          return (
            fs.existsSync(log) &&
            fs.readFileSync(log, "utf8").trim().split("\n").length === 2
          );
        },
        { timeout: 15_000 },
      );
      const starts = fs
        .readFileSync(path.join(root, "applies.jsonl"), "utf8")
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line));
      assert.deepEqual(
        new Set(starts.map((start) => start.repo)),
        new Set(repos),
      );
      assert.ok(
        Math.abs(starts[1].at - starts[0].at) < 20_000,
        "providers did not overlap",
      );
      await $("#repository-result").selectByAttribute("value", "1");
      assert.equal(await $("#apply-vendor").getValue(), vendors[0]);
      assert.equal(await $("#apply-model input").getValue(), models[0]);
      await expect($("#apply-fixes")).toBeDisabled();
      await expect($("#run")).toBeDisabled();
      for (const [index, vendor] of vendors.entries()) {
        assert.ok(
          (await $("#apply-jobs").getText()).includes(
            `${vendor}:${models[index]}`,
          ),
        );
        assert.ok(starts[index].args.includes(models[index]));
        assert.match(
          fs.readFileSync(path.join(repos[index], "src/pricing.rs"), "utf8"),
          /if quantity > 50/,
        );
      }
      await browser.waitUntil(
        () =>
          fs
            .readFileSync(path.join(repos[0], "src/pricing.rs"), "utf8")
            .includes("if quantity >= 50"),
        { timeout: 30_000 },
      );
      await expect($("#output")).toHaveText(
        /Fixed the bulk discount threshold/,
      );
      await expect($("#run")).toBeDisabled();
      await expect($("#stop")).toBeDisplayed();
      await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
        timeout: 45_000,
      });
      for (const [index, repo] of repos.entries()) {
        assert.match(
          fs.readFileSync(path.join(repo, "src/pricing.rs"), "utf8"),
          /if quantity >= 50/,
        );
        await $("#repository-result").selectByAttribute(
          "value",
          String(index + 1),
        );
        await expect($("#output")).toHaveText(
          /Fixed the bulk discount threshold/,
        );
        assert.equal(await $("#apply-vendor").getValue(), vendors[index]);
      }
      const settingsFile = path.join(
        process.env["APPDATA"]!,
        "BugSleuth/settings.json",
      );
      await browser.waitUntil(
        () =>
          JSON.parse(fs.readFileSync(settingsFile, "utf8"))
            .apply_repositories?.[repos[1]]?.apply_model ===
          `${vendors[1]}:${models[1]}`,
        { timeout: 10_000 },
      );
    });
  }
});
