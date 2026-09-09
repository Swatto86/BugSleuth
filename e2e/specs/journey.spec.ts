/** Persistence across a native restart, then an observed clean exit. */
import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { appPids } from "../workspace.ts";
import { MODEL, REPO, configureOneSweep, treeDigest } from "./support.ts";

const application =
  process.env["BUGSLEUTH_E2E_APPLICATION"] ??
  path.resolve(
    `target/debug/bugsleuth-app${process.platform === "win32" ? ".exe" : ""}`,
  );

describe("native persistence and exit", () => {
  it("persists the chosen model and theme across a restart", async () => {
    await browser.setWindowSize(1100, 760);
    if ((await $$("#matrix-body tr")).length === 0)
      await $("#add-model").click();
    await $("#repo").setValue(REPO);
    await configureOneSweep(MODEL);
    await $("#theme").selectByAttribute("value", "light");
    const settings = path.join(
      process.env["APPDATA"]!,
      "BugSleuth/settings.json",
    );
    await browser.waitUntil(
      async () => {
        const saved = JSON.parse(fs.readFileSync(settings, "utf8"));
        return saved.theme === "light" && saved.models[0]?.id === MODEL;
      },
      {
        timeout: 10_000,
        timeoutMsg: "UI changes were not saved to isolated settings",
      },
    );
    const layout = await browser.execute(() => {
      const main = document.querySelector("main")!;
      const model = document.querySelector("#matrix-body td.model-id input")!;
      const width = main.clientWidth;
      const content = main.scrollWidth;
      const matrixScroll = document.querySelector(".matrix-scroll")!;
      matrixScroll.scrollLeft = matrixScroll.scrollWidth;
      const remove = document.querySelector(
        "#matrix-body tr:last-child button",
      )!;
      return {
        width,
        content,
        modelWidth: model.getBoundingClientRect().width,
        lastControlReachable:
          remove.getBoundingClientRect().right <=
          matrixScroll.getBoundingClientRect().right + 1,
        afterScrollWidth: main.scrollWidth,
        matrixWidth: matrixScroll.clientWidth,
        matrixOverflow: getComputedStyle(matrixScroll).overflowX,
        matrixContain: getComputedStyle(matrixScroll).contain,
        overflowing: [...main.querySelectorAll("*")]
          .filter(
            (el) =>
              el.getBoundingClientRect().right >
              main.getBoundingClientRect().right + 1,
          )
          .map((el) => `${el.tagName}#${el.id}.${el.className}`)
          .slice(0, 10),
      };
    });
    assert.ok(
      layout.content <= layout.width + 1,
      `The page overflows horizontally: ${JSON.stringify(layout)}`,
    );
    assert.ok(
      layout.modelWidth >= 160,
      `Model ID field is squeezed to ${layout.modelWidth}px`,
    );
    assert.ok(
      layout.lastControlReachable,
      "The matrix's last control cannot be reached by scrolling",
    );
    const before = treeDigest(REPO);
    await browser.saveScreenshot(
      path.join(process.env["BUGSLEUTH_E2E_ROOT"]!, "before-restart.png"),
    );
    await browser.reloadSession();
    await browser.waitUntil(async () => await $("#repo").isExisting(), {
      timeout: 30_000,
    });
    assert.equal(await $("#repo").getValue(), REPO);
    assert.equal(await $("#theme").getValue(), "light");
    assert.equal(
      await $("#matrix-body tr:first-child td.model-id input").getValue(),
      MODEL.slice(MODEL.indexOf(":") + 1),
    );
    assert.equal(treeDigest(REPO), before);
  });

  it("exits the owned app through its normal Quit button", async () => {
    const pids = appPids(application);
    assert.equal(pids.length, 1, "expected exactly one owned app before Quit");
    let clickError: unknown;
    try {
      await $("#quit").click();
    } catch (error) {
      clickError = error;
    }
    await browser.waitUntil(async () => appPids(application).length === 0, {
      timeout: 15_000,
      timeoutMsg: `Quit did not terminate the app: ${String(clickError ?? "")}`,
    });
  });
});
