import fs from "node:fs";

describe("probe", () => {
  it("dumps what the webview has", async () => {
    await browser.pause(5000);
    const url = await browser.getUrl().catch((e) => `ERR ${e}`);
    const title = await browser.getTitle().catch((e) => `ERR ${e}`);
    const src = await browser.getPageSource().catch((e) => `ERR ${e}`);
    fs.writeFileSync(
      "probe-result.json",
      JSON.stringify(
        { url, title, len: String(src).length, head: String(src).slice(0, 1000).replace(/\s+/g, " ") },
        null,
        1,
      ),
    );
  });
});
