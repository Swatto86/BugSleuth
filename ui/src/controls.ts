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

import { type Settings, boundedProviderConcurrency } from "./model";
import { signinPills } from "./view";

/** The elements these handlers touch, and what they do to the rest. */
export interface ControlDeps {
  ui: {
    theme: HTMLSelectElement;
    repo: HTMLInputElement;
    scope: HTMLInputElement;
    providerConcurrency: HTMLInputElement;
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
  ui.providerConcurrency.addEventListener("input", () => {
    settings().provider_concurrency = boundedProviderConcurrency(
      ui.providerConcurrency.value,
    );
    refresh();
  });
  // On commit, snap the field to the value the run will actually use: the input
  // handler clamps but never writes the clamp back, so the box could show 50
  // while the run fans out to 10. max="10" does not clamp typed input, so
  // reconcile on `change` (blur), not `input`, so it does not fight typing.
  ui.providerConcurrency.addEventListener("change", () => {
    const bounded = boundedProviderConcurrency(ui.providerConcurrency.value);
    ui.providerConcurrency.value = String(bounded);
    settings().provider_concurrency = bounded;
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
