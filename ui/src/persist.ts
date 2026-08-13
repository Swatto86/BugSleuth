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
  save?: (settings: Settings) => Promise<void>;
}

export interface SettingsSaver {
  /** Permit writes. Called once, after the stored settings really loaded. */
  allowWrites: () => void;
  /** Refuse writes for the session, and say why in the persistent region. */
  blockWrites: (reason: string) => void;
  schedule: () => void;
  flush: () => Promise<boolean>;
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
export function savingSettings(deps: PersistDeps): SettingsSaver {
  let timer: ReturnType<typeof globalThis.setTimeout> | undefined;
  let dirty = false;
  let failed = false;
  // Closed until the stored settings are known to have loaded. `boot` used to
  // continue with in-memory defaults after a failed `load_settings`, and the
  // first later edit atomically replaced the unreadable — but often salvageable
  // — file with those defaults, before any warning had even been shown.
  let writable = false;
  let tail: Promise<boolean> = Promise.resolve(true);

  const cancelTimer = (): void => {
    if (timer !== undefined) globalThis.clearTimeout(timer);
    timer = undefined;
  };
  const enqueue = (): Promise<boolean> => {
    const snapshot = structuredClone(deps.settings());
    dirty = false;
    tail = tail.then(async () => {
      try {
        if (deps.save) await deps.save(snapshot);
        else await invoke<void>("save_settings", { settings: snapshot });
        if (failed) deps.setError("");
        failed = false;
        return true;
      } catch (error) {
        failed = true;
        deps.setError(`Settings are not being saved: ${String(error)}`);
        return false;
      }
    });
    return tail;
  };

  return {
    allowWrites: () => {
      writable = true;
    },
    blockWrites: (reason) => {
      writable = false;
      failed = true;
      cancelTimer();
      deps.setError(`Settings are not being saved: ${reason}`);
    },
    schedule: () => {
      dirty = true;
      cancelTimer();
      if (!writable) return;
      timer = globalThis.setTimeout(() => {
        timer = undefined;
        void enqueue();
      }, 400);
    },
    flush: async () => {
      cancelTimer();
      // Reported as "not saved" rather than as a clean flush, so Quit's existing
      // failed-flush confirmation still asks before discarding the edits.
      if (!writable) return !dirty;
      while (true) {
        const pending = dirty ? enqueue() : tail;
        const ok = await pending;
        cancelTimer();
        if (!dirty && pending === tail) return ok;
      }
    },
  };
}
