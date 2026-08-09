import { strict as assert } from "node:assert";

describe("parallel review agents", () => {
  it("offers them only for providers that can delegate read-only", async () => {
    await browser.waitUntil(
      async () => (await (await $$("#matrix-body tr")).length) > 0,
      {
        timeout: 30_000,
        timeoutMsg: "the model matrix never rendered",
      },
    );
    const provider = () =>
      $("#matrix-body tr:first-child td:first-child select");
    const agents = () =>
      $("#matrix-body tr:first-child td.agent-cell input[type=checkbox]");
    const effort = () =>
      $("#matrix-body tr:first-child td.effort-cell select");

    await provider().selectByAttribute("value", "claude");
    await browser.waitUntil(
      async () =>
        await $(
          "#matrix-body tr:first-child datalist option[value=fable]",
        ).isExisting(),
      {
        timeout: 70_000,
        timeoutMsg: "the Claude catalogue never offered Fable",
      },
    );
    await expect(agents()).toBeEnabled();
    await agents().click();
    await expect(agents()).toBeSelected();
    await expect(effort()).toBeDisabled();
    assert.equal(await effort().getText(), "Ultracode");

    await provider().selectByAttribute("value", "kilo");
    await expect(agents()).toBeDisabled();
    await expect(agents()).not.toBeSelected();
    assert.match(
      (await agents().getAttribute("title")) ?? "",
      /cannot delegate/,
    );

    await provider().selectByAttribute("value", "codex");
    await expect(agents()).toBeEnabled();
    await expect(agents()).not.toBeSelected();
  });
});
