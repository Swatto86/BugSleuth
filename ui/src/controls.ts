/**
 * The settings controls, wired to state.
 *
 * Every one of these is the same shape: read a control, write the matching
 * field, re-render. They sit apart from `main.ts` for the same reason the
 * guarded actions do — those need a confirmation, these do not, and neither
 * needs to be read while working out how the window boots.
 *
 * State is handed in rather than imported. `main.ts` owns the one mutable
 * settings object; this file is given a reader and the redraw functions, so
 * there is no second place that can decide what the settings are.
 */

import { invoke } from "@tauri-apps/api/core";

import { type Settings, boundedProveTop } from "./model";
import { signinPills } from "./view";

/** The elements these handlers touch, and what they do to the rest. */
export interface ControlDeps {
  ui: {
    theme: HTMLSelectElement;
    repo: HTMLInputElement;
    scope: HTMLInputElement;
    proveEnabled: HTMLInputElement;
    proveSettings: HTMLDivElement;
    proveTop: HTMLInputElement;
    testCommand: HTMLInputElement;
    reuseCompleted: HTMLInputElement;
    triageSeverities: HTMLInputElement;
    browse: HTMLButtonElement;
    checkSignin: HTMLButtonElement;
    vendors: HTMLDivElement;
    addModel: HTMLButtonElement;
    copyPrompt: HTMLButtonElement;
  };
  settings: () => Settings;
  /** The model that re-grades severities, or "" when the pass is off. */
  triageModel: string;
  applyTheme: (theme: Settings["theme"]) => void;
  /** Redraw what depends on state but not on the table's identity. */
  refresh: () => void;
  /** Rebuild the model table. */
  render: () => void;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  fixPrompt: () => string;
}

export function bindControls(deps: ControlDeps): void {
  const { ui, refresh, render } = deps;
  const settings = deps.settings;

  ui.theme.addEventListener("change", () => {
    settings().theme = ui.theme.value as Settings["theme"];
    deps.applyTheme(settings().theme);
    refresh();
  });

  ui.repo.addEventListener("input", () => {
    settings().repo = ui.repo.value;
    refresh();
  });
  ui.scope.addEventListener("input", () => {
    settings().scope = ui.scope.value;
    refresh();
  });
  // Proof is a decision, so it gets a switch. It used to be expressed only as
  // a number that meant "off" at zero — and a number field showing 0 does not
  // read as off, it reads as a number you have not filled in yet. The first
  // person to open the app could not tell whether proving was on, or how to
  // turn it off, which matters more here than for most settings: proving runs
  // the reviewed repository's own build and test commands on this machine.
  const showProof = () => {
    const on = ui.proveEnabled.checked;
    ui.proveSettings.classList.toggle("hidden", !on);
    // The count is the switch's state as far as Rust is concerned: zero is off.
    settings().prove_top = on ? boundedProveTop(ui.proveTop.value) || 1 : 0;
  };

  ui.proveEnabled.addEventListener("change", () => {
    showProof();
    refresh();
  });

  ui.proveTop.addEventListener("input", () => {
    settings().prove_top = ui.proveEnabled.checked
      ? boundedProveTop(ui.proveTop.value) || 1
      : 0;
    refresh();
  });
  ui.testCommand.addEventListener("input", () => {
    settings().test_command = ui.testCommand.value;
    refresh();
  });
  ui.reuseCompleted.addEventListener("change", () => {
    settings().reuse_completed = ui.reuseCompleted.checked;
    refresh();
  });
  ui.triageSeverities.addEventListener("change", () => {
    // Off is an empty model rather than a separate flag, so there is one thing
    // to read on the Rust side and no way for the two to disagree.
    settings().triage_model = ui.triageSeverities.checked
      ? deps.triageModel
      : "";
    refresh();
  });

  ui.browse.addEventListener("click", () => {
    // Not an `async` listener. addEventListener throws away the promise it gets
    // back, so an `await` that rejects inside one has no caller to propagate to
    // and reaches nobody: the folder picker did nothing, said nothing, and left
    // someone clicking a button that appeared to be broken.
    invoke<string | null>("pick_directory")
      .then((picked) => {
        if (!picked) return;
        settings().repo = picked;
        ui.repo.value = picked;
        refresh();
      })
      .catch((error: unknown) => {
        deps.setStatus(
          `Could not open the folder picker: ${String(error)}`,
          "error",
        );
      });
  });

  ui.checkSignin.addEventListener("click", () => {
    // Disabled while it runs: each click is three real model calls, and a
    // second one started on top of the first would spend twice to learn once.
    // Eir guards its own provider test the same way, for the same reason.
    ui.checkSignin.disabled = true;
    const previous = ui.checkSignin.textContent;
    ui.checkSignin.textContent = "Asking each vendor…";
    invoke<{ name: string; available: boolean; detail: string }[]>(
      "check_signin",
    )
      .then((results) => {
        ui.vendors.replaceChildren(...signinPills(results));
        const working = results.filter((r) => r.available).length;
        deps.setStatus(
          `${working} of ${results.length} vendors answered`,
          working === 0 ? "error" : "",
        );
      })
      .catch((error: unknown) => {
        deps.setStatus(`Could not check sign-in: ${String(error)}`, "error");
      })
      .finally(() => {
        ui.checkSignin.disabled = false;
        ui.checkSignin.textContent = previous;
      });
  });

  ui.addModel.addEventListener("click", () => {
    settings().models = [
      ...settings().models,
      { id: "", lanes: [], effort: "", passes: 1 },
    ];
    render();
  });

  ui.copyPrompt.addEventListener("click", () => {
    void navigator.clipboard.writeText(deps.fixPrompt()).then(
      () => {
        // Confirm by changing the button, not with a dialog: you are about to
        // paste somewhere else, and a dialog would be one more thing to dismiss.
        ui.copyPrompt.textContent = "Copied";
        window.setTimeout(() => {
          ui.copyPrompt.textContent = "Copy fix prompt";
        }, 1500);
      },
      () => {
        // Clipboard access can be refused. Say so and point at the file, which
        // is always written — silently doing nothing would be the worst outcome
        // for the one button that hands over the result.
        ui.copyPrompt.textContent = "Copy failed — use the saved file";
        ui.copyPrompt.classList.add("error");
      },
    );
  });
}
