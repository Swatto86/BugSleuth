import { strict as assert } from "node:assert";

import { clickDialogButton } from "./support.ts";

describe("dialog focus", () => {
  it("keeps version and updates in About without browser chrome", async () => {
    await $("#about").click();
    await expect($("#about-dialog")).toBeDisplayed();
    assert.match(await $("#app-version").getText(), /^Version \d+\.\d+\.\d+$/);
    await expect($("#check-update")).toBeDisplayed();

    const contextMenuPrevented = await browser.execute(() => {
      const event = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
      });
      document.querySelector("#app-version")?.dispatchEvent(event);
      return event.defaultPrevented;
    });
    assert.equal(contextMenuPrevented, true);

    const headings = await browser.execute(() =>
      [...document.querySelectorAll("#matrix th")]
        .slice(0, 5)
        .map((heading) => heading.textContent?.trim()),
    );
    assert.deepEqual(headings, [
      "Provider",
      "Model",
      "Agents",
      "Effort",
      "Passes",
    ]);

    await $("#about-close").click();
    await expect($("#about-dialog")).not.toBeDisplayed();
    assert.equal(
      await browser.execute(() => document.activeElement?.id),
      "about",
    );
  });

  it("falls back to status when its opener disappears", async () => {
    if ((await $$("#matrix-body tr")).length === 0) {
      await $("#add-model").click();
    }

    await $("#matrix-body tr:last-child button").click();
    await expect($(".dialog-overlay .dialog")).toBeDisplayed();
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
