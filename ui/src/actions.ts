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
import { isApplying } from "./apply";

let clearing = false;
export const isClearing = (): boolean => clearing;

export interface ActionDeps {
  ui: {
    run: HTMLButtonElement;
    stop: HTMLButtonElement;
    quit: HTMLButtonElement;
    clearSaved: HTMLButtonElement;
    /** Hidden when the prompt it would apply is deleted. */
    applyPanel: HTMLDivElement;
    promptPath: HTMLParagraphElement;
  };
  settings: () => Settings;
  setSettings: (models: Settings["models"]) => void;
  render: () => void;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  focusStatus: () => void;
  updating: () => boolean;
  activityChanged: () => void;
  flushSettings: () => Promise<boolean>;
  runDeps: () => RunDeps;
  /** Whether the whole matrix is exactly one shipped preset. */
  isShippedConfiguration: (models: Settings["models"]) => boolean;
}

export function bindGuardedActions(deps: ActionDeps): void {
  const { ui } = deps;
  const el = <T extends HTMLElement>(id: string): T => {
    const found = document.getElementById(id);
    if (!found) throw new Error(`missing element #${id}`);
    return found as T;
  };

  // The frontend's own view of a clear_saved invoke in flight. The backend
  // counts clearing as work that must not be killed silently (tray.rs), but the
  // window had no matching state, so a quit during a delete — window Quit or the
  // tray's, which routes through this window — exited mid-`remove_dir_all` with
  // no warning, leaving the runs directory half-deleted.
  let quitting = false;

  for (const name of ["cheap", "balanced", "deep"] as Preset[]) {
    el<HTMLButtonElement>(`preset-${name}`).addEventListener("click", () => {
      // A preset replaces the whole matrix. Sitting beside "Add model", it is
      // an easy misclick, and there is no undo — several minutes of chosen
      // vendors, lanes, efforts and passes would simply be gone.
      //
      // Asked only when something would actually be lost: a confirmation that
      // appears when nothing is at stake is how people learn to click through
      // confirmations without reading them.
      if (deps.isShippedConfiguration(deps.settings().models)) {
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

  // Sweeps cost real quota, so throwing them away is asked about — but the
  // question is worth asking for the opposite reason too. Reuse is on by
  // default and silent, and a sweep taken yesterday reads exactly like one
  // taken now: a run reused sweeps from the night before, hours after the
  // defects they described had been fixed, and produced a fix prompt for a
  // repository that no longer existed.
  ui.clearSaved.addEventListener("click", () => {
    void confirmDialog({
      title: "Delete the saved sweeps?",
      message:
        "This removes every stored sweep and fix prompt for this repository. " +
        "They cost subscription quota and cannot be recovered — the next run " +
        "pays for them again, and reviews the code as it is now.",
      confirmLabel: "Delete them",
      destructive: true,
    }).then((yes) => {
      if (!yes) return;
      // Flagged for the whole delete, so a quit while it runs is warned about
      // rather than exiting mid-delete. Cleared on both outcomes below — a flag
      // left set on either path would warn about a clear that had finished.
      // Focus leaves first. `activityChanged()` reaches renderPlanSummary,
      // which disables this very button — and in WebView2 disabling the focused
      // element drops focus to <body>, so the check below it was already false
      // and a keyboard user spent the whole delete at the document root.
      // `clearing` is set before either, so Run and Clear are still correctly
      // disabled; only the synchronous status and focus work moves ahead.
      clearing = true;
      deps.setStatus("Deleting the saved sweeps…", "running");
      if (document.activeElement === ui.clearSaved) deps.focusStatus();
      deps.activityChanged();
      ui.clearSaved.disabled = true;
      invoke<{ removed: number }>("clear_saved", {
        settings: deps.settings(),
      })
        .then((cleared) => {
          clearing = false;
          deps.activityChanged();
          ui.clearSaved.disabled = false;
          // The prompt this panel would apply has just been deleted, so the
          // button would fail with "run a review first". Offering a control
          // that cannot work is the defect this app exists to find.
          ui.applyPanel.classList.add("hidden");
          ui.promptPath.classList.add("hidden");
          deps.setStatus(
            cleared.removed === 0
              ? "Nothing was stored for this repository — the next run starts fresh either way"
              : `Deleted ${cleared.removed} saved file${cleared.removed === 1 ? "" : "s"}. The next run sweeps from scratch.`,
          );
        })
        .catch((error: unknown) => {
          clearing = false;
          deps.activityChanged();
          ui.clearSaved.disabled = false;
          // Silence here would read as "cleared", and the next run would then
          // quietly reuse everything this was meant to remove.
          deps.setStatus(
            `Could not clear the saved sweeps: ${String(error)}`,
            "error",
          );
        });
    });
  });

  // Closing the window only hides it. This is the reachable, keyboard-navigable
  // way to actually exit, so the tray is not the single point of failure.
  //
  // Guarded while a run is in flight: a sweep is tens of minutes of paid
  // subscription quota, and quitting throws away every lane not yet written to
  // disk. One misplaced click should not be able to do that silently.
  ui.stop.addEventListener("click", () => {
    const applying = isApplying();
    void confirmDialog({
      title: applying ? "Stop applying the fixes?" : "Stop this review?",
      message: applying
        ? "The model is killed part-way through editing this repository. " +
          "Commits it has already made are kept — check `git status` and " +
          "`git log` afterwards to see how far it got."
        : "Sweeps that have already finished are kept on disk, and running " +
          "again with reuse enabled picks up from there rather than paying for " +
          "them twice. Sweeps still in flight are abandoned.",
      confirmLabel: applying ? "Stop applying" : "Stop the review",
      destructive: true,
    }).then((yes) => {
      if (!yes) return;
      // Checked again, after the dialog. A review takes tens of minutes and the
      // dialog can sit open for any part of it, so the run may have finished on
      // its own while it did — and this used to overwrite "Finished" and the
      // ranked defects' status with a permanent, false "Stopping…".
      if (applying ? !isApplying() : !isRunning()) return;
      deps.setStatus(
        applying
          ? "Stopping — killing the model mid-apply"
          : "Stopping — killing the sweeps in flight",
        "running",
      );
      if (document.activeElement === ui.stop) deps.focusStatus();
      ui.stop.disabled = true;
      // A failed cancel must not leave the button dead and the status stuck on
      // "Stopping" forever, with a run still burning quota behind it. Offer the
      // button back and say what happened.
      void (applying ? invoke("cancel_apply") : invoke("cancel_run")).catch(
        (error: unknown) => {
          ui.stop.disabled = false;
          deps.setStatus(
            applying
              ? `Could not stop the apply: ${String(error)}`
              : `Could not stop the run: ${String(error)}`,
            "error",
          );
        },
      );
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

  // One handled route out, for both the immediate and post-confirmation quits.
  // Remaining visible is not feedback after someone pressed Quit: if the IPC
  // rejects, say so in the live status region rather than looking broken.
  function requestQuit(acknowledged = false): void {
    if (quitting) return;
    quitting = true;
    deps.setStatus("Saving settings before quitting…", "running");
    if (document.activeElement === ui.quit) deps.focusStatus();
    ui.quit.disabled = true;
    const abortQuit = (): void => {
      quitting = false;
      ui.quit.disabled = false;
    };
    void (async () => {
      if (!(await deps.flushSettings())) {
        deps.setStatus("Latest settings could not be saved", "error");
        const discard = await confirmDialog({
          title: "Quit without saving settings?",
          message:
            "The latest settings could not be saved. Quitting now discards those changes.",
          confirmLabel: "Quit without saving",
          destructive: true,
        });
        if (!discard) {
          abortQuit();
          return;
        }
      }
      // Checked again, after the flush: the settings save can take long enough
      // for a run, apply, clear or update to start, and an unacknowledged quit
      // must not kill it silently.
      if (
        !acknowledged &&
        (isRunning() || isApplying() || clearing || deps.updating())
      ) {
        abortQuit();
        askBeforeQuitting();
        return;
      }
      try {
        deps.setStatus("Quitting…", "running");
        await invoke("quit");
      } catch (error) {
        abortQuit();
        deps.setStatus(`Could not quit BugSleuth: ${String(error)}`, "error");
      }
    })().catch((error: unknown) => {
      abortQuit();
      deps.setStatus(
        `Could not save settings before quitting: ${String(error)}`,
        "error",
      );
    });
  }

  function askBeforeQuitting(): void {
    if (!isRunning() && !isApplying() && !clearing && !deps.updating()) {
      requestQuit();
      return;
    }
    // Every active operation gets its own consequence rather than one generic
    // warning that hides whether code, saved results, or an install is at risk.
    const applying = isApplying();
    const running = isRunning();
    const updating = deps.updating();
    void confirmDialog({
      title: updating
        ? "An update is being installed"
        : applying
          ? "Fixes are being applied"
          : running
            ? "A review is running"
            : "Saved sweeps are being cleared",
      message: updating
        ? "Quitting interrupts installation before BugSleuth restarts. Let it finish first."
        : applying
          ? "Quitting kills the model part-way through editing this repository. " +
            "Some files may already have changed and nothing will be left to say " +
            "which — check `git status` afterwards."
          : running
            ? "Quitting abandons it. Every sweep not yet written to disk is lost, " +
              "along with the subscription quota it cost. Stop the review instead " +
              "if you want to keep what has finished."
            : "Quitting part-way through deleting the saved sweeps would leave " +
              "some removed and some not, and it cannot be undone. Let it finish " +
              "first.",
      confirmLabel: "Quit anyway",
      destructive: true,
    }).then((yes) => {
      if (yes) requestQuit(true);
    });
  }
}
