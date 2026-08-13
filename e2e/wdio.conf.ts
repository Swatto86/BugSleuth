/**
 * WebDriver harness for the real BugSleuth window.
 *
 * This drives the **release binary** through its actual webview, not a mock and
 * not the Vite dev server. Everything cheaper is already covered elsewhere:
 * Rust tests for the engine's rules, `node --test` for the frontend's state and
 * wording. What only this can see is the boundary between them — that the
 * window really appears, that a click really reaches Rust, and that closing
 * really hides instead of exiting.
 *
 * Deliberately small. `~/.agents/tauri.md` is explicit that an exhaustive
 * feature-per-spec WebDriver suite is not the goal; a short journey that boots
 * the app, exercises the main action and exits cleanly is.
 */

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { assertVerificationSucceeded } from "./driver-security.ts";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");

/** The packaged binary. Built by `npx tauri build` — see the check below. */
const application = path.resolve(root, "target/release/bugsleuth-app.exe");

/**
 * Sanity-check the binary before spending a session on it.
 *
 * **`cargo build --release` does not produce a shippable app.** It embeds the
 * dev-server URL rather than the frontend, so with Vite running it looks
 * perfect and without it the window is blank — and every check that looks at
 * exit codes, or at whether a window appeared, passes either way. Only
 * `cargo tauri build` bundles the frontend in.
 *
 * Worse, cargo caches the result: once a plain release build has produced a
 * dev-configured artefact, a later `tauri build` may see nothing changed and
 * reuse it. `cargo clean -p bugsleuth-app` first if in doubt.
 *
 * Two further traps met while getting this working, recorded so nobody repeats
 * them: `npx tauri build` fails with "could not determine executable to run"
 * unless the npm CLI package is installed — the cargo subcommand is what works
 * here — and it fails *quietly* enough to look like a successful no-op.
 */
function assertProductionBuild(exe: string): void {
  if (!fs.existsSync(exe)) {
    throw new Error(`no binary at ${exe} — build it with: npx tauri build`);
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
        `Rebuild with: cargo tauri build`,
    );
  }
}

let tauriDriver: ChildProcess | undefined;

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
  specs: [path.resolve(here, "specs/**/*.spec.ts")],
  maxInstances: 1,
  logLevel: "error",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    // A Tauri window plus a WebView2 cold start is not fast, and one spec runs
    // a *real* review — a genuine model call against a real repository, which
    // this project documents as taking minutes. 120s passed it sometimes and
    // failed it others, which is the worst kind of test.
    timeout: 420_000,
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
    } catch (error) {
      console.error(`
${String(error instanceof Error ? error.message : error)}
`);
      process.exit(1);
    }
    // Any copy the user left open would win the single-instance guard, and the
    // driver-launched app would exit on the spot — a session attached to
    // nothing, which is indistinguishable from the app being broken. The suite
    // drives the shipped binary, guard included, so it clears the field first.
    await assertDriverPortFree();
    spawnSync("taskkill", ["/IM", path.basename(application), "/T", "/F"], {
      stdio: "ignore",
    });
    await new Promise((resolve) => setTimeout(resolve, 1500));

    // Verified here, not only where setup downloaded it. Setup can be skipped
    // entirely, and the file can be replaced afterwards, so the only check that
    // protects this execution is the one immediately before it.
    const nativeDriver = path.resolve(root, ".webdriver/msedgedriver.exe");
    assertMicrosoftDriver(nativeDriver);
    tauriDriver = spawn("tauri-driver", ["--native-driver", nativeDriver], {
      stdio: [null, process.stdout, process.stderr],
      shell: false,
    });
    await waitForDriver();
  },

  onComplete: () => {
    // Kill the whole tree: tauri-driver spawns msedgedriver, which spawns the
    // app. Killing only the parent leaves both behind holding port 4444, and
    // the next run then fails for a reason that has nothing to do with the app.
    if (tauriDriver?.pid) {
      spawnSync("taskkill", ["/pid", String(tauriDriver.pid), "/T", "/F"], {
        stdio: "ignore",
      });
    }
  },
};
