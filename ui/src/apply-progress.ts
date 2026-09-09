/**
 * How far the fixes have got, and how far an interrupted run had already got.
 *
 * Two answers about the same thing, from two places. The live one arrives as
 * events while a fix run is working through the defects; the other is read off
 * the engine's own journal, because a run that ended when the usage allowance
 * ran out is exactly the one someone comes back to after closing the app, and a
 * count held only in the window would be gone by then.
 *
 * Kept out of `apply.ts`, which is about choosing who fixes the defects. This
 * is about what happened to the last attempt.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ApplyProgressDeps {
  /** The repository whose report is on screen; others are tracked silently. */
  promptRepo: () => string;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  /** Redraw whatever depends on the counts — in practice, the button's label. */
  onChange: () => void;
}

export interface ApplyProgress {
  /** Defects a stopped or failed run already fixed and committed. */
  alreadyFixed: (repo: string) => number;
  /**
   * Ask Rust again, because the journal on disk or the fixing model has
   * changed. The model matters: a journal written under another model is one
   * the engine starts over from, and the label must not promise otherwise.
   */
  refresh: (repo: string, model: string) => void;
}

export function bindApplyProgress(deps: ApplyProgressDeps): ApplyProgress {
  /**
   * Defects an interrupted fix run for this repository already finished.
   *
   * Read from the engine's own journal rather than remembered in the window: a
   * fix run that ended because the usage allowance ran out is exactly the one
   * the user comes back to after closing the app, and a number held only in
   * memory would be gone by then. Absent until the first answer arrives, so the
   * button never briefly claims a resume it cannot do.
   */
  const alreadyFixed = new Map<string, number>();

  const refresh = (repo: string, model: string): void => {
    if (repo === "") return;
    invoke<number | null>("unfinished_apply", { repo, model })
      .then((done) => {
        if (done && done > 0) alreadyFixed.set(repo, done);
        else alreadyFixed.delete(repo);
        deps.onChange();
      })
      .catch(() => {
        // A journal that cannot be read is not an error worth interrupting
        // anyone for: the button keeps its ordinary label and Apply still
        // resumes, because the engine reads the same file for itself.
        alreadyFixed.delete(repo);
      });
  };

  /**
   * How far the fixes have got, defect by defect.
   *
   * A fix run is minutes to hours. Without this the window shows one spinner
   * for all of it, so a long defect and a stuck one look the same and Stop is a
   * decision made blind about how much work would be thrown away.
   */
  void listen<{
    kind: "started" | "defect_started" | "defect_finished";
    repo: string;
    defects: number;
    already?: number;
    done?: number;
    position?: number;
  }>("apply-progress", (event) => {
    const { repo, kind, defects } = event.payload;
    if (repo !== deps.promptRepo()) return;
    if (kind === "started") {
      const already = event.payload.already ?? 0;
      deps.setStatus(
        already > 0
          ? `Resuming the fixes: ${already} of ${defects} defects are already done`
          : `Applying the fixes: ${defects} defect${defects === 1 ? "" : "s"} to work through`,
        "running",
      );
      return;
    }
    const done = event.payload.done ?? 0;
    if (kind === "defect_started") {
      deps.setStatus(`Fixing defect ${done + 1} of ${defects}`, "running");
      return;
    }
    // Recorded as done on disk at this point, so the number is what a resume
    // would actually skip rather than an optimistic count.
    alreadyFixed.set(repo, done);
    deps.setStatus(`Fixed ${done} of ${defects} defects`, "running");
  });

  return {
    alreadyFixed: (repo) => alreadyFixed.get(repo) ?? 0,
    refresh,
  };
}
