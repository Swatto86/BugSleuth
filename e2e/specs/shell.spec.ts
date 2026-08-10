/**
 * The shell itself: theming, and where the keyboard is left after the matrix
 * rebuilds. Split from `review.spec.ts` at the hard line cap — those specs are
 * about the lifecycle of a review, and these are about the window around it.
 */

import { strict as assert } from "node:assert";

import { clickDialogButton } from "./support.ts";

describe("BugSleuth shell", () => {
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
});
