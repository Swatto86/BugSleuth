/** Signed automatic updates, deferred until repository work is quiet. */
import { invoke } from "@tauri-apps/api/core";

interface Available {
  version: string;
  current: string;
  notes: string;
}

export interface UpdateDeps {
  button: HTMLButtonElement;
  notice: HTMLElement;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  focusStatus: () => void;
  busy: () => boolean;
  flushSettings: () => Promise<boolean>;
  setSettingsLocked: (locked: boolean) => void;
  activityChanged: () => void;
}

let updating = false;
export const isUpdating = (): boolean => updating;

export function wireUpdate(deps: UpdateDeps): () => void {
  const { button, setStatus } = deps;
  let checking = false;
  let stopped = false;
  let pending: Available | null = null;
  let deferred: ReturnType<typeof setTimeout> | undefined;
  const label = button.textContent;
  const announce = (message: string): void => {
    deps.notice.textContent = message;
    deps.notice.classList.remove("hidden");
  };

  function disableButton(): void {
    if (document.activeElement === button) deps.focusStatus();
    button.disabled = true;
  }

  async function installWhenQuiet(): Promise<void> {
    if (stopped || updating || !pending) return;
    if (deps.busy()) {
      announce(
        `Version ${pending.version} is ready. BugSleuth will update after the current work finishes.`,
      );
      deferred = setTimeout(() => void installWhenQuiet(), 1000);
      return;
    }
    const update = pending;
    pending = null;
    updating = true;
    deps.focusStatus();
    deps.setSettingsLocked(true);
    deps.activityChanged();
    disableButton();
    try {
      setStatus("Saving settings before installing…", "running");
      if (!(await deps.flushSettings())) {
        const message = `Version ${update.version} was not installed because the latest settings could not be saved`;
        announce(message);
        setStatus(message, "error");
        return;
      }
      announce(
        `Installing ${update.version}. BugSleuth will restart automatically.`,
      );
      setStatus(`Installing ${update.version}…`, "running");
      // Rust checks the signed manifest again and atomically refuses if a
      // review, apply or clear operation won the race to start.
      await invoke("install_update");
    } catch (error: unknown) {
      const message = `Could not install ${update.version}: ${String(error)}`;
      announce(message);
      setStatus(message, "error");
    } finally {
      updating = false;
      deps.setSettingsLocked(false);
      deps.activityChanged();
      button.disabled = false;
      button.textContent = label;
    }
  }

  async function check(manual = false): Promise<void> {
    if (stopped || checking || updating || pending) return;
    checking = true;
    disableButton();
    button.textContent = "Checking…";
    if (manual) {
      setStatus("Checking for updates…", "running");
    }
    try {
      pending = await invoke<Available | null>("check_for_update");
      if (stopped) {
        pending = null;
        return;
      }
      if (pending) {
        await installWhenQuiet();
      } else if (manual) {
        setStatus("You are on the latest version");
      }
    } catch (error: unknown) {
      const message = `Could not check for updates: ${String(error)}`;
      announce(message);
      if (manual) setStatus(message, "error");
    } finally {
      checking = false;
      if (!updating) {
        button.disabled = false;
        button.textContent = label;
      }
    }
  }

  const clicked = (): void => {
    void check(true);
  };
  button.addEventListener("click", clicked);
  void check();
  const timer = setInterval(() => void check(), 4 * 60 * 60 * 1000);
  return () => {
    stopped = true;
    clearInterval(timer);
    clearTimeout(deferred);
    button.removeEventListener("click", clicked);
  };
}
