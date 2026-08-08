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
  /** Show a save failure in its own persistent region, or clear it with "". */
  setError: (text: string) => void;
}

/**
 * A save function, coalesced so typing does not write on every keystroke.
 *
 * The failure goes to its own persistent live region, not the transient status
 * bar. The status bar was suppressed whenever a run was in progress — the exact
 * time a save is most likely to be attempted and to fail — so a whole run's
 * worth of configuration could be lost with nothing ever said. This region only
 * ever carries the save state, and is cleared only by a later successful save.
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
          deps.setError("");
        })
        .catch((error: unknown) => {
          failed = true;
          deps.setError(`Settings are not being saved: ${String(error)}`);
        });
    }, 400);
  };
}
