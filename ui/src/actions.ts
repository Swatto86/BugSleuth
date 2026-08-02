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
      ui.stop.disabled = true;
      deps.setStatus("Stopping — killing the sweeps in flight", "running");
      void invoke("cancel_run");
    });
  });

  ui.quit.addEventListener("click", () => {
    if (!isRunning()) {
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
      if (yes) void invoke("quit");
    });
  });
}
