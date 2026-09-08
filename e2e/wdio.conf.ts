/** Drives the real debug binary with isolated settings and subprocess fixtures. */

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { prepareWorkspace, appPids, providerPids, live } from "./workspace.ts";
import { assertVerificationSucceeded } from "./driver-security.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");

const application =
  process.env["BUGSLEUTH_E2E_APPLICATION"] ??
  path.resolve(
    root,
    `target/debug/bugsleuth-app${process.platform === "win32" ? ".exe" : ""}`,
  );

function assertProductionBuild(exe: string): void {
  if (!fs.existsSync(exe)) {
    throw new Error(
      `no binary at ${exe} — build it with: npx tauri build --debug --no-bundle`,
    );
  }
  // Freshness, not build kind. Whether the frontend is embedded cannot be seen
  // by inspection — Tauri compresses the bundled assets, so grepping the binary
  // for markup finds nothing either way, and the config it embeds contains
  // devUrl in *both* kinds. The reliable signal is behavioural, and the suite
  // itself provides it: the first spec fails immediately on a dev binary,
  // because a blank page has no elements.
  //
  // What is worth checking cheaply is that the binary is not older than the
  // frontend it is supposed to contain, which is the mistake that actually
  // happens: edit the UI, forget to rebuild, then debug the old one.
  const built = fs.statSync(exe).mtimeMs;
  const distIndex = path.resolve(root, "ui/dist/index.html");
  if (fs.existsSync(distIndex) && fs.statSync(distIndex).mtimeMs > built) {
    throw new Error(
      `${exe} is older than ui/dist — it does not contain the current frontend. ` +
        `Rebuild with: npx tauri build --debug --no-bundle`,
    );
  }
}

let tauriDriver: ChildProcess | undefined;

function stopDriver(): void {
  if (tauriDriver?.pid) {
    const pid = tauriDriver.pid;
    if (process.platform === "win32") {
      const result = spawnSync(
        "taskkill",
        ["/pid", String(tauriDriver.pid), "/T", "/F"],
        { stdio: "pipe" },
      );
      if (result.status !== 0 && tauriDriver.exitCode === null)
        throw new Error("Owned driver cleanup failed");
    } else {
      for (const provider of providerPids(application)) {
        try {
          process.kill(-provider, "SIGTERM");
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
        }
      }
      try {
        process.kill(-pid, "SIGTERM");
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
      }
    }
  }
  tauriDriver = undefined;
}

const driverStatus = () =>
  fetch("http://127.0.0.1:4444/status", {
    signal: AbortSignal.timeout(1_000),
  });

/** Refuse to mistake somebody else's driver for the one this run owns. */
async function assertDriverPortFree(): Promise<void> {
  try {
    await driverStatus();
  } catch {
    return;
  }
  throw new Error(
    "port 4444 is already held by a WebDriver process; stop the stale tauri-driver.exe before rerunning E2E",
  );
}

/**
 * Wait until the driver is actually accepting connections.
 *
 * `spawn` returning is not the driver being ready — it takes a moment to bind
 * 4444, and wdio's first request goes out immediately. The suite failed with
 * "Unable to connect to http://127.0.0.1:4444/", which reads like a missing
 * driver and is really a race with a driver that was seconds from ready.
 *
 * Worse than the failure was one variant of it: with a stale driver already
 * holding the port from an earlier aborted run, the suite hung with no output
 * at all rather than failing, and sat there for over an hour. So this also
 * fails loudly with the reason rather than waiting forever.
 */
async function waitForDriver(): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError = "no attempt made";
  while (Date.now() < deadline) {
    if (tauriDriver?.exitCode !== null && tauriDriver?.exitCode !== undefined) {
      throw new Error(
        `tauri-driver exited with code ${tauriDriver.exitCode} before binding 4444`,
      );
    }
    try {
      // Any answer at all means it is listening. WebDriver replies 404 to a
      // bare GET, which is a perfectly good sign of life.
      await driverStatus();
      return;
    } catch (error) {
      lastError = String(error);
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  throw new Error(
    `tauri-driver never accepted a connection on 127.0.0.1:4444 within 30s. ` +
      `Last error: ${lastError}. A driver left running by an aborted run will ` +
      `hold that port - check for a stray tauri-driver.exe.`,
  );
}

/**
 * Refuse to run a native driver Windows cannot prove Microsoft signed.
 *
 * Shares the setup script's verifier rather than reimplementing the check, so
 * the two sinks cannot come to disagree about what "verified" means.
 */
function assertMicrosoftDriver(driver: string): void {
  const verifier = path.resolve(root, "scripts/assert-microsoft-signature.ps1");
  const checked = spawnSync(
    "pwsh",
    ["-NoProfile", "-NonInteractive", "-File", verifier, "-Path", driver],
    { stdio: "pipe", windowsHide: true, shell: false },
  );
  assertVerificationSucceeded(checked);
}

export const config: WebdriverIO.Config = {
  runner: "local",
  framework: "mocha",
  specs: [
    ["review", "agents", "dialog-focus", "shell", "apply", "journey"].map(
      (name) => path.resolve(here, `specs/${name}.spec.ts`),
    ),
  ],
  maxInstances: 1,
  logLevel: "error",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    // A Tauri window plus a WebView2 cold start is not fast, and one spec runs
    // a *real* review — a genuine model call against a real repository, which
    // has its own 14-minute wait. WebdriverIO wraps each test at this timeout,
    // so changing the timeout inside the test does not extend the outer guard.
    timeout: 15 * 60_000,
  },

  // tauri-driver listens here and proxies to msedgedriver.
  hostname: "127.0.0.1",
  port: 4444,

  capabilities: [
    {
      // @ts-expect-error tauri:options is a tauri-driver capability, not a
      // standard WebDriver one, so it is absent from WebdriverIO's types.
      "tauri:options": { application },
      browserName: "wry",
      // WebdriverIO 9 adds `webSocketUrl: true` to every capability set unless
      // told otherwise, asking for a WebDriver BiDi session. tauri-driver
      // rewrites only `tauri:options` and proxies everything else verbatim, so
      // that flag reaches msedgedriver on a path nobody tests.
      "wdio:enforceWebDriverClassic": true,
    },
  ],

  onPrepare: async () => {
    // **Fatal, not logged.** WebdriverIO reports a throwing `onPrepare` and
    // then starts the workers anyway, so a clear message — "the binary does not
    // contain the current frontend, rebuild it" — became an unrelated
    // ECONNREFUSED from a driver that was never started. The first time this
    // happened the suite produced no output at all and sat for over an hour.
    //
    // The check is worth keeping precisely because it fires often: the gate
    // runs `vite build`, which makes `ui/dist` newer than the app, so any gate
    // run after a package build leaves the binary stale.
    try {
      assertProductionBuild(application);
      await assertDriverPortFree();
      if (appPids(application).length > 0) {
        throw new Error(
          "The selected BugSleuth binary is already running; E2E will not terminate it.",
        );
      }
      prepareWorkspace(root);
      const nativeDriver =
        process.platform === "win32"
          ? path.resolve(root, ".webdriver/msedgedriver.exe")
          : (process.env["PATH"] ?? "")
              .split(path.delimiter)
              .map((dir) => path.resolve(dir, "WebKitWebDriver"))
              .find((candidate) => fs.existsSync(candidate));
      if (!nativeDriver)
        throw new Error("Install WebKitWebDriver before running Linux E2E");
      if (process.platform === "win32") assertMicrosoftDriver(nativeDriver);
      tauriDriver = spawn("tauri-driver", ["--native-driver", nativeDriver], {
        stdio: [null, process.stdout, process.stderr],
        shell: false,
        env: {
          ...process.env,
          ...(process.platform === "linux"
            ? { GDK_BACKEND: "x11", WEBKIT_DISABLE_DMABUF_RENDERER: "1" }
            : {}),
        },
        detached: process.platform !== "win32",
      });
      process.once("exit", stopDriver);
      await waitForDriver();
    } catch (error) {
      if (tauriDriver?.pid) tauriDriver.kill();
      console.error(error);
      process.exit(1);
    }
  },

  onComplete: () => {
    stopDriver();
    console.log(
      `E2E evidence (${live ? "live" : "fixture"}): ${process.env["BUGSLEUTH_E2E_ROOT"]}`,
    );
  },
};
