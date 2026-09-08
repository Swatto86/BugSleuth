/** Release acceptance: real Codex sessions writing two disposable repositories. */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { live } from "../workspace.ts";
import { providerPids } from "../workspace.ts";
import {
  MODEL,
  REPO,
  configureOneSweep,
  clickDialogButton,
  providerCliProcesses,
} from "./support.ts";

describe("live simultaneous Codex fixes", () => {
  it("fixes both scanned repositories with overlapping real Codex processes", async function () {
    if (!live) this.skip();
    await $("#check-signin").waitForEnabled({ timeout: 60_000 });
    const root = process.env["BUGSLEUTH_E2E_ROOT"]!;
    const repos = ["live-codex-first", "live-codex-second"].map((name) =>
      path.join(root, name),
    );
    const acceptance =
      "#[test]\nfn discount_starts_at_fifty() {\n    assert_eq!(seeded_repo::pricing::basket_total(100, 50), 4500);\n}\n";
    // Kept outside the writable repositories: added provider tests are welcome,
    // but cannot change the behavior this independent check requires.
    const check = (repo: string, fixed: boolean): void => {
      const source = path.join(root, `${path.basename(repo)}-acceptance.rs`);
      const binary = source.replace(
        /\.rs$/,
        process.platform === "win32" ? ".exe" : "",
      );
      fs.writeFileSync(
        source,
        `#[path = ${JSON.stringify(path.join(repo, "src/pricing.rs").replaceAll("\\", "/"))}]\nmod pricing;\n${acceptance.replace("seeded_repo::", "")}`,
      );
      execFileSync(
        "rustc",
        ["--edition=2021", "--test", source, "-o", binary],
        { stdio: "pipe" },
      );
      const result = spawnSync(binary, [], { encoding: "utf8" });
      assert.equal(
        result.status,
        fixed ? 0 : 101,
        result.stdout + result.stderr,
      );
      assert.match(
        result.stdout,
        fixed
          ? /discount_starts_at_fifty \.\.\. ok/
          : /discount_starts_at_fifty \.\.\. FAILED/,
      );
    };
    for (const repo of repos) {
      fs.cpSync(REPO, repo, {
        recursive: true,
        filter: (file) => ![".git", "target"].includes(path.basename(file)),
      });
      fs.mkdirSync(path.join(repo, "tests"), { recursive: true });
      fs.writeFileSync(path.join(repo, "tests/fix_acceptance.rs"), acceptance);
      const git = (...args: string[]) =>
        execFileSync("git", ["-C", repo, ...args], { stdio: "pipe" });
      git("init", "-b", "main");
      git("config", "user.name", "Acceptance");
      git("config", "user.email", "acceptance@localhost");
      git("add", ".");
      git("commit", "-m", "Fixture");
      check(repo, false);
    }
    await $("#repo").setValue(repos[0]);
    await $("#additional-repos").setValue(repos[1]);
    await configureOneSweep(MODEL);
    await $("#run").click();
    await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
      timeout: 10 * 60_000,
    });
    await expect($("#fix-board")).toBeDisplayed();
    await $("#repository-result").selectByAttribute("value", "1");
    fs.writeFileSync(
      path.join(root, "scan-result.txt"),
      await $("#output").getText(),
    );
    await browser.saveScreenshot(path.join(root, "live-scan.png"));
    for (let index = 0; index < 2; index++) {
      const row = `.fix-repository:nth-child(${index + 1})`;
      await $(`${row} [data-fix-vendor]`).selectByAttribute("value", "codex");
      // Empty selects the authenticated CLI's own default model.
      assert.ok(
        await $(`${row} [data-fix-start]`).isEnabled(),
        await $("#output").getText(),
      );
      await $(`${row} [data-fix-start]`).click();
      await clickDialogButton("Apply the fixes");
    }
    await browser.waitUntil(
      () =>
        providerPids(
          process.env["BUGSLEUTH_E2E_APPLICATION"] ??
            path.resolve(
              `target/debug/bugsleuth-app${process.platform === "win32" ? ".exe" : ""}`,
            ),
        ).length >= 2,
      {
        timeout: 30_000,
      },
    );
    await browser.saveScreenshot(path.join(root, "live-codex-overlap.png"));
    await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
      timeout: 10 * 60_000,
    });
    for (const [index, repo] of repos.entries()) {
      check(repo, true);
      const tests = execFileSync("cargo", ["test", "--quiet"], {
        cwd: repo,
        encoding: "utf8",
      });
      assert.match(tests, /test result: ok/);
      await $("#repository-result").selectByAttribute(
        "value",
        String(index + 1),
      );
      await expect($("#output")).toHaveText(/src\/pricing.rs/);
    }
    await browser.waitUntil(() => providerCliProcesses().length === 0, {
      timeout: 15_000,
    });
    assert.ok(
      fs
        .readFileSync(path.join(REPO, "src/pricing.rs"), "utf8")
        .includes("if quantity > 50"),
      "the unrelated source fixture changed",
    );
  });
});
