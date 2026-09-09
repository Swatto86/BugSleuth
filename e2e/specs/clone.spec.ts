/** Real Git through the webview, including failure and persisted selection. */
import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { REPO, MODEL, RUNS_ROOT, configureOneSweep } from "./support.ts";

describe("cloning repositories", () => {
  it("adds multiple clones without replacing existing repositories", async () => {
    const parent = path.dirname(REPO);
    const sources = ["source-one", "source-two"].map((name) =>
      path.join(parent, name),
    );
    for (const source of sources)
      execFileSync("git", ["clone", "--", REPO, source], { stdio: "pipe" });
    const destination = path.join(parent, "multiple-clones");
    fs.mkdirSync(destination);
    await $("#clone-open").click();
    await browser.execute((text: string) => {
      const input = document.getElementById(
        "clone-source",
      ) as HTMLTextAreaElement;
      input.value = text;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }, sources.join("\n"));
    await $("#clone-parent").setValue(destination);
    await $("#clone-name").clearValue();
    await browser.saveScreenshot(path.join(parent, "multiple-clones.png"));
    await $("#clone-start").click();
    await browser.waitUntil(
      async () => !(await $("#clone-dialog").isDisplayed()),
      { timeout: 30_000 },
    );
    const expected = [
      REPO,
      ...sources.map((source) => path.join(destination, path.basename(source))),
    ];
    assert.equal(await $("#repo").getValue(), expected.join("\n"));
    for (const repo of expected)
      assert.ok(fs.existsSync(path.join(repo, "Cargo.toml")));
    await browser.saveScreenshot(path.join(parent, "repositories-to-scan.png"));
    await $("#repo").setValue(REPO);
  });
  it("protects existing folders and selects a full cloned checkout", async () => {
    const parent = path.dirname(REPO);
    const destination = path.join(parent, "cloned repo");
    await $("#clone-open").click();
    await $("#clone-source").setValue(REPO);
    await $("#clone-parent").setValue(parent);
    await $("#clone-name").setValue(path.basename(REPO));
    await $("#clone-start").click();
    await browser.waitUntil(async () =>
      (await $("#clone-status").getText()).includes("already exist"),
    );
    assert.equal(await $("#repo").getValue(), REPO);
    assert.equal(
      await browser.execute(() => document.activeElement?.id),
      "clone-close",
    );
    await browser.saveScreenshot(path.join(parent, "clone-dialog.png"));
    await $("#clone-name").setValue("cloned repo");
    await $("#clone-start").click();
    await browser.waitUntil(
      async () => !(await $("#clone-dialog").isDisplayed()),
      { timeout: 30_000 },
    );
    assert.equal(await $("#repo").getValue(), [REPO, destination].join("\n"));
    const git = (repo: string, args: string[]) =>
      execFileSync("git", ["-C", repo, ...args], { encoding: "utf8" }).trim();
    assert.equal(
      git(destination, ["rev-parse", "HEAD"]),
      git(REPO, ["rev-parse", "HEAD"]),
    );
    assert.ok(fs.existsSync(path.join(destination, "Cargo.toml")));
    assert.equal(git(destination, ["remote", "get-url", "origin"]), REPO);
    const settings = path.join(
      process.env["APPDATA"]!,
      "BugSleuth/settings.json",
    );
    await browser.waitUntil(
      async () =>
        JSON.parse(fs.readFileSync(settings, "utf8")).additional_repos?.[0] ===
        destination,
    );
    await configureOneSweep(MODEL);
    await $("#run").click();
    await browser.waitUntil(
      async () =>
        (await $("#status").getText()).startsWith("Review incomplete"),
      { timeout: 60_000 },
    );
    const runFolder = fs
      .readdirSync(RUNS_ROOT)
      .find((folder) => folder.startsWith("cloned repo-"));
    assert.ok(runFolder, "No review output for the cloned repository");
    assert.ok(
      fs
        .readdirSync(path.join(RUNS_ROOT, runFolder))
        .some((file) => file.endsWith(".json")),
    );
    await $("#repo").setValue(REPO);
  });
  it("stops a live Git transport and leaves the partial destination unselected", async () => {
    const sockets = new Set<net.Socket>();
    const server = net.createServer((socket) => {
      sockets.add(socket);
      socket.resume(); // Drain the SSH banner so Node can observe EOF on cancellation.
      socket.on("close", () => sockets.delete(socket));
      socket.on("error", () => socket.destroy());
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address() as net.AddressInfo;
    try {
      await $("#clone-open").click();
      await $("#clone-source").setValue(
        `ssh://git@127.0.0.1:${address.port}/waiting.git`,
      );
      await $("#clone-name").setValue("stopped-clone");
      await $("#clone-start").click();
      await browser.waitUntil(async () => sockets.size > 0, {
        timeout: 15_000,
      });
      assert.equal(
        await browser.execute(() => document.activeElement?.id),
        "clone-close",
      );
      await $("#clone-close").click();
      await browser.waitUntil(async () =>
        (await $("#clone-status").getText()).includes("Clone stopped"),
      );
      await browser.waitUntil(async () => sockets.size === 0);
      assert.equal(await $("#repo").getValue(), REPO);
      await $("#clone-close").click();
    } finally {
      for (const socket of sockets) socket.destroy();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      if (await $("#clone-dialog").isDisplayed()) {
        await browser.waitUntil(
          async () => await $("#clone-start").isEnabled(),
        );
        await $("#clone-close").click();
      }
    }
  });
});

describe("authenticated Git clone acceptance", () => {
  const source = process.env["BUGSLEUTH_E2E_CLONE_URL"];
  (source ? it : it.skip)(
    "clones a private remote using the existing Git credentials",
    async () => {
      const parent = path.dirname(REPO);
      const destination = path.join(parent, "private-clone");
      await $("#clone-open").click();
      await $("#clone-source").setValue(source!);
      await $("#clone-parent").setValue(parent);
      await $("#clone-name").setValue("private-clone");
      await $("#clone-start").click();
      await browser.waitUntil(
        async () => !(await $("#clone-dialog").isDisplayed()),
        { timeout: 180_000, timeoutMsg: "Authenticated clone did not finish" },
      );
      assert.equal(await $("#repo").getValue(), [REPO, destination].join("\n"));
      const files = execFileSync("git", ["-C", destination, "ls-files", "-z"], {
        encoding: "utf8",
      })
        .split("\0")
        .filter(Boolean);
      assert.ok(files.length > 0);
      assert.ok(
        files.some((file) => fs.existsSync(path.join(destination, file))),
      );
      await $("#repo").setValue(REPO);
    },
  );
});
