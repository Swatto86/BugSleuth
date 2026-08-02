/**
 * The actions that ask before they act.
 *
 * Grouped because they share one rule, learned from the app reviewing itself:
 * a confirmation is owed when something would actually be lost, and owed
 * *only* then. A dialog that appears when nothing is at stake is how people
 * learn to click through dialogs without reading them, which is worse than
 * having none.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { confirmDialog } from "./dialog";
import { type Preset, type Settings, preset } from "./model";
import { type RunDeps, isRunning, startRun } from "./run";

export interface ActionDeps {
  ui: {
    run: HTMLButtonElement;
    stop: HTMLButtonElement;
    quit: HTMLButtonElement;
  };
  settings: () => Settings;
  setSettings: (models: Settings["models"]) => void;
  render: () => void;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  runDeps: () => RunDeps;
  /** Whether a row is still exactly as some preset shipped it. */
  isShipped: (model: Settings["models"][number]) => boolean;
}

export function bindGuardedActions(deps: ActionDeps): void {
  const { ui } = deps;
  const el = <T extends HTMLElement>(id: string): T => {
    const found = document.getElementById(id);
    if (!found) throw new Error(`missing element #${id}`);
    return found as T;
  };

  for (const name of ["cheap", "balanced", "deep"] as Preset[]) {
    el<HTMLButtonElement>(`preset-${name}`).addEventListener("click", () => {
      // A preset replaces the whole matrix. Sitting beside "Add model", it is
      // an easy misclick, and there is no undo — several minutes of chosen
      // vendors, lanes, efforts and passes would simply be gone.
      //
      // Asked only when something would actually be lost: a confirmation that
      // appears when nothing is at stake is how people learn to click through
      // confirmations without reading them.
      if (deps.settings().models.every(deps.isShipped)) {
        deps.setSettings(preset(name));
        deps.render();
        return;
      }
      void confirmDialog({
        title: "Replace your configuration?",
        message:
          "This replaces every row in the matrix with the " +
          `${name} preset, and cannot be undone.`,
        confirmLabel: "Replace",
        destructive: true,
      }).then((yes) => {
        if (!yes) return;
        deps.setSettings(preset(name));
        deps.render();
      });
    });
  }

  ui.run.addEventListener("click", () => void startRun(deps.runDeps()));

  // Closing the window only hides it. This is the reachable, keyboard-navigable
  // way to actually exit, so the tray is not the single point of failure.
  //
  // Guarded while a run is in flight: a sweep is tens of minutes of paid
  // subscription quota, and quitting throws away every lane not yet written to
  // disk. One misplaced click should not be able to do that silently.
  ui.stop.addEventListener("click", () => {
    void confirmDialog({
      title: "Stop this review?",
      message:
        "Sweeps that have already finished are kept on disk, and running " +
        "again with reuse enabled picks up from there rather than paying for " +
        "them twice. Sweeps still in flight are abandoned.",
      confirmLabel: "Stop the review",
      destructive: true,
    }).then((yes) => {
      if (!yes) return;
      // Checked again, after the dialog. A review takes tens of minutes and the
      // dialog can sit open for any part of it, so the run may have finished on
      // its own while it did — and this used to overwrite "Finished" and the
      // ranked defects' status with a permanent, false "Stopping…".
      if (!isRunning()) return;
      ui.stop.disabled = true;
      deps.setStatus("Stopping — killing the sweeps in flight", "running");
      // A failed cancel must not leave the button dead and the status stuck on
      // "Stopping" forever, with a run still burning quota behind it. Offer the
      // button back and say what happened.
      void invoke("cancel_run").catch((error: unknown) => {
        ui.stop.disabled = false;
        deps.setStatus(`Could not stop the run: ${String(error)}`, "error");
      });
    });
  });

  // The tray's Quit asks the window to put the question, so both routes out of
  // the app get the same warning and the wording lives in one place. The tray
  // used to call exit(0) outright, which is the *more* likely of the two to be
  // used — the README calls it the only real exit — and was the one that could
  // throw away a run in silence.
  void listen("confirm-quit", () => {
    askBeforeQuitting();
  }).catch((error: unknown) => {
    deps.setStatus(
      `The tray's Quit cannot reach this window: ${String(error)}`,
      "error",
    );
  });

  ui.quit.addEventListener("click", askBeforeQuitting);

  function askBeforeQuitting(): void {
    if (!isRunning()) {
      // invoke-may-fail-silently: if quitting fails the window is still here,
      // which is the whole feedback there is to give, and there is no fallback
      // to offer beyond the tray item that does the same thing.
      void invoke("quit");
      return;
    }
    void confirmDialog({
      title: "A review is running",
      message:
        "Quitting abandons it. Every sweep not yet written to disk is lost, " +
        "along with the subscription quota it cost. Stop the review instead " +
        "if you want to keep what has finished.",
      confirmLabel: "Quit anyway",
      destructive: true,
    }).then((yes) => {
      // invoke-may-fail-silently: as above — a failed quit leaves a visible
      // window, and the run it was abandoning carries on being reported.
      if (yes) void invoke("quit");
    });
  }
}
