import { strict as assert } from "node:assert";

import { clickDialogButton } from "./support.ts";

describe("dialog focus", () => {
  it("falls back to status when its opener disappears", async () => {
    if ((await $$("#matrix-body tr")).length === 0) {
      await $("#add-model").click();
    }

    await $("#matrix-body tr:last-child button").click();
    await expect($(".dialog")).toBeDisplayed();
    await browser.execute(() => {
      const opener = document.querySelector(
        "#matrix-body tr:last-child button",
      ) as HTMLElement | null;
      if (opener) opener.hidden = true;
    });

    await clickDialogButton("Cancel");
    assert.equal(
      await browser.execute(() => document.activeElement?.id),
      "status",
    );
  });
});
