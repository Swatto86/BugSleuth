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

import { spawn, type ChildProcess } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");

/** The release binary. Built by `cargo build --release -p bugsleuth-app`. */
const application = path.resolve(root, "target/release/bugsleuth-app.exe");

let tauriDriver: ChildProcess | undefined;

export const config: WebdriverIO.Config = {
  runner: "local",
  framework: "mocha",
  specs: [path.resolve(here, "specs/**/*.spec.ts")],
  maxInstances: 1,
  logLevel: "error",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    // A Tauri window plus a WebView2 cold start is not fast.
    timeout: 120_000,
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
    },
  ],

  onPrepare: () => {
    tauriDriver = spawn(
      "tauri-driver",
      ["--native-driver", path.resolve(root, ".webdriver/msedgedriver.exe")],
      { stdio: [null, process.stdout, process.stderr], shell: true },
    );
  },

  onComplete: () => {
    // Kill the whole tree: tauri-driver spawns msedgedriver, which spawns the
    // app. Killing only the parent leaves both behind holding port 4444, and
    // the next run then fails for a reason that has nothing to do with the app.
    if (tauriDriver?.pid) {
      spawn("taskkill", ["/pid", String(tauriDriver.pid), "/T", "/F"], { shell: true });
    }
  },
};
