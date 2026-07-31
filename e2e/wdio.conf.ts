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
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
    assertProductionBuild(application);
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
