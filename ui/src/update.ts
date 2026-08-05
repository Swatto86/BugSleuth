/**
 * Checking for a newer release, and installing it.
 *
 * A window rather than a background task, because installing an update
 * replaces the running executable and restarts the app. Doing that under
 * someone mid-review would throw away a run that costs real quota — so the
 * check is a button, and the install is a confirmation.
 *
 * The download and signature check happen in Rust. The page can ask, and is
 * told the answer; it cannot fetch or execute anything itself, which is why
 * the updater's frontend permission is deliberately not granted.
 */

import { invoke } from "@tauri-apps/api/core";

import { confirmDialog } from "./dialog";

interface Available {
  version: string;
  current: string;
  notes: string;
}

export interface UpdateDeps {
  button: HTMLButtonElement;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  /** True while work that an install-and-restart would interrupt is in flight. */
  busy: () => boolean;
}

export function wireUpdate(deps: UpdateDeps): void {
  const { button, setStatus } = deps;

  button.addEventListener("click", () => {
    button.disabled = true;
    const previous = button.textContent;
    button.textContent = "Checking…";

    invoke<Available | null>("check_for_update")
      .then(async (update) => {
        if (!update) {
          setStatus("You are on the latest version");
          return;
        }

        // Asked, not assumed. An install restarts the app, and a review or an
        // apply in flight would be killed part-way.
        if (deps.busy()) {
          setStatus(
            `Version ${update.version} is available — finish the current operation first`,
            "error",
          );
          return;
        }

        const notes = update.notes.trim();
        const agreed = await confirmDialog({
          title: `Version ${update.version} is available`,
          message:
            `You have ${update.current}. Installing downloads the new version, ` +
            `replaces this one and restarts BugSleuth.` +
            (notes ? `\n\n${notes.slice(0, 600)}` : ""),
          confirmLabel: "Install and restart",
        });
        if (!agreed) return;

        setStatus(`Installing ${update.version}…`, "running");
        try {
          // Nothing after this resolves: the process is replaced. An error is
          // the only thing that can come back, and it needs its own message —
          // reporting a failed install as "could not check for updates" sends
          // the reader looking at their network instead of at the install.
          await invoke("install_update");
        } catch (error: unknown) {
          setStatus(
            `Could not install ${update.version}: ${String(error)}`,
            "error",
          );
        }
      })
      .catch((error: unknown) => {
        // Said out loud rather than swallowed. A check that fails silently is
        // how an install sits eight releases behind while looking current.
        setStatus(`Could not check for updates: ${String(error)}`, "error");
      })
      .finally(() => {
        button.disabled = false;
        button.textContent = previous;
      });
  });
}
