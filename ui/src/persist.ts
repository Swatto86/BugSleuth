/**
 * Keeping settings on disk, and saying so when that stops working.
 *
 * Split from the wiring because it carries the one rule in this file that was
 * learned rather than designed: a save failure must be reported.
 *
 * It used to be swallowed, on the reasoning that losing a preference is not
 * worth interrupting anyone over. That was wrong in a way that took hours to
 * unpick — saving failed for a whole session, the app looked perfectly normal,
 * and every configuration made in it was gone on restart. The cost of a wrong
 * preference is small; the cost of not knowing your settings are not being kept
 * is not.
 */

import { invoke } from "@tauri-apps/api/core";

import type { Settings } from "./model";

export interface PersistDeps {
  settings: () => Settings;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  /** True while something more important is on the status bar - a run. */
  quiet: () => boolean;
}

/**
 * A save function, coalesced so typing does not write on every keystroke.
 *
 * The failure is reported in the status bar rather than a dialog, because it
 * must not interrupt a run in progress, and only while nothing else is being
 * said. Recovery is reported too: a status line stuck on an error that has
 * since cleared is its own small lie.
 */
export function savingSettings(deps: PersistDeps): () => void {
  let timer: number | undefined;
  let failed = false;

  return () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      void invoke("save_settings", { settings: deps.settings() })
        .then(() => {
          if (!failed) return;
          failed = false;
          if (!deps.quiet()) deps.setStatus("Ready");
        })
        .catch((error: unknown) => {
          failed = true;
          if (!deps.quiet()) {
            deps.setStatus(`Settings are not being saved: ${String(error)}`, "error");
          }
        });
    }, 400);
  };
}
