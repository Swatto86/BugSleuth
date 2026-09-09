/** Real read-only Astra review: source access, findings and unchanged files. */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { live } from "../workspace.ts";
import { REPO, configureOneSweep, runsDir, treeDigest } from "./support.ts";

describe("live Astra repository review", () => {
  it("reads the repository and reports a seeded defect without changing source", async function () {
    if (!live) this.skip();
    await $("#check-signin").waitForEnabled({ timeout: 60_000 });
    const before = treeDigest(REPO);
    await configureOneSweep("codex:gpt-6-astra");
    await $("#run").click();
    await browser.waitUntil(async () => !(await $("#stop").isDisplayed()), {
      timeout: 10 * 60_000,
    });
    const output = await $("#output").getText();
    const dir = runsDir();
    assert.ok(dir, output);
    const files = fs
      .readdirSync(dir)
      .filter((name) => name.includes("astra") && name.endsWith(".json"));
    assert.equal(files.length, 1, output);
    const report = JSON.parse(
      fs.readFileSync(path.join(dir, files[0]!), "utf8"),
    );
    assert.equal(report.status.state, "swept", JSON.stringify(report));
    assert.ok(
      report.findings.length > 0,
      "Astra found none of the seeded defects",
    );
    assert.ok(
      report.findings.some((finding: { anchor: { file: string } }) =>
        finding.anchor.file.startsWith("src/"),
      ),
      JSON.stringify(report),
    );
    assert.equal(
      treeDigest(REPO),
      before,
      "the read-only review changed source",
    );
    await browser.saveScreenshot(
      path.join(process.env["BUGSLEUTH_E2E_ROOT"]!, "astra-review.png"),
    );
  });
});
