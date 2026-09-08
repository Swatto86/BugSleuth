/** Observe a real repository write through the native Apply controls. */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { live } from "../workspace.ts";
import {
  MODEL,
  REPO,
  clickDialogButton,
  configureOneSweep,
} from "./support.ts";

describe("native apply", () => {
  it("applies the displayed finding and reports the changed file", async () => {
    if ((await $$("#matrix-body tr")).length === 0)
      await $("#add-model").click();
    await $("#repo").setValue(REPO);
    await configureOneSweep(MODEL);
    await $("#run").click();
    await browser.waitUntil(
      async () =>
        (await $("#status").getText()).startsWith("Review incomplete"),
      { timeout: 14 * 60_000, interval: 2000 },
    );
    await $("#apply-vendor").selectByAttribute(
      "value",
      MODEL.includes(":") ? MODEL.split(":")[0] : "claude",
    );
    await $("#apply-model input").setValue(
      MODEL.includes(":") ? MODEL.slice(MODEL.indexOf(":") + 1) : MODEL,
    );
    if (await $("#push-after-apply").isSelected())
      await $("#push-after-apply").click();
    const file = path.join(REPO, "src/pricing.rs");
    const before = fs.readFileSync(file, "utf8");
    await $("#apply-fixes").click();
    await clickDialogButton("Apply the fixes");
    await browser.waitUntil(
      async () => (await $("#status").getText()).startsWith("Fixes applied"),
      { timeout: 14 * 60_000, interval: 2000 },
    );
    const after = fs.readFileSync(file, "utf8");
    assert.notEqual(
      after,
      before,
      "Apply reported success without changing the file",
    );
    if (!live)
      assert.equal(
        after,
        before.replace("if quantity > 50", "if quantity >= 50"),
      );
    assert.match(await $("#output").getText(), /src\/pricing.rs/);
  });
  it("refuses another apply while preserving uncommitted work", async () => {
    const file = path.join(REPO, "src/pricing.rs");
    fs.appendFileSync(file, "\n// Unsaved acceptance work must survive.\n");
    const before = fs.readFileSync(file, "utf8");
    await $("#apply-fixes").click();
    await clickDialogButton("Apply the fixes");
    await browser.waitUntil(
      async () =>
        (await $("#status").getText()).startsWith("Applying the fixes failed"),
      { timeout: 15_000 },
    );
    assert.match(await $("#output").getText(), /uncommitted changes/);
    assert.equal(fs.readFileSync(file, "utf8"), before);
  });
});
