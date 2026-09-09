/**
 * Choosing who fixes the defects, and setting them to work.
 *
 * The one control in this app that changes the user's own code. Everything else
 * reads; this writes, so it asks first, it says exactly what it is about to do,
 * and when it is done it shows what git observed rather than what the model
 * claimed.
 *
 * Kept out of `run.ts` deliberately: that file owns the review's lifecycle, and
 * this is what happens *after* one. The only thing they share is the output
 * pane, which this appends to rather than replaces — the ranked defects are
 * what you read the applied changes against.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { applyConfirmation } from "./apply-confirm";
import { confirmDialog } from "./dialog";
import { bindFixBoard } from "./fix-board";
import {
  applyStatus,
  effortIsValid,
  joinId,
  settingsForApply,
  splitId,
  type Settings,
  type Vendor,
} from "./model";
import { offeredVendors, vendorCliPresent } from "./cli-offer.ts";
import { effortPicker, modelPicker, option } from "./pickers";
import { bindApplyProgress } from "./apply-progress";
import type { Catalogue } from "./view";

import {
  activeApplies,
  applyLog,
  recordApply,
  repositoryDeps,
} from "./apply-repositories";

const NEWLINE = String.fromCharCode(10);

export interface ApplyDeps {
  ui: {
    vendor: HTMLSelectElement;
    /** Holds the model box, which is rebuilt when the provider changes. */
    model: HTMLDivElement;
    /** Holds the effort control, rebuilt whenever the model changes. */
    effort: HTMLDivElement;
    button: HTMLButtonElement;
    stop: HTMLButtonElement;
    /** Opt-in to publishing what the apply commits. */
    push: HTMLInputElement;
    /** Opt-in to tagging what the push published, so CI releases it. */
    tag: HTMLInputElement;
    output: HTMLPreElement;
    /** Persistent alert for a completion listener that failed to register. */
    listenerError: HTMLParagraphElement;
  };
  settings: () => Settings;
  promptRepo: () => string;
  /** The vendor menus, which arrive after the window has already drawn. */
  catalogue: () => Catalogue;
  /** Whether a review is in flight; applying during one is refused. */
  busy: () => boolean;
  /** Save settings and redraw whatever depends on them. */
  refresh: () => void;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  focusStatus: () => void;
}

export const isApplying = (): boolean => activeApplies.size > 0;

export interface ApplyBinding {
  redraw: () => void;
  refreshButton: () => void;
}

/** Wire the panel up and expose its two focused redraw operations. */
export function bindApply(deps: ApplyDeps): ApplyBinding {
  const board = bindFixBoard(deps, () => draw());
  deps = repositoryDeps(deps);
  const { ui } = deps;

  let completionEventsReady = false;

  // How far each repository's fixes have got, and how far an interrupted run
  // had got before it stopped. Kept beside the panel rather than inside it
  // because both answers come from Rust — one live, one off disk — and the only
  // thing this file does with them is label a button.
  const fixes = bindApplyProgress({
    promptRepo: () => deps.promptRepo(),
    setStatus: deps.setStatus,
    onChange: () => setButtonState(),
  });

  /** Draw the provider, model and effort controls from the stored settings. */
  const draw = (): void => {
    board.draw();
    const focused = document.activeElement;
    const focusKey =
      focused instanceof HTMLElement &&
      (ui.model.contains(focused) || ui.effort.contains(focused))
        ? focused.dataset["focusKey"]
        : undefined;
    const stored = deps.settings().apply_model;
    const selected = splitId(stored).vendor;
    ui.vendor.replaceChildren(
      ...offeredVendors(deps.catalogue(), selected).map((name) =>
        option(name, name, name === selected),
      ),
    );
    ui.vendor.value = selected;
    if (ui.vendor.value !== selected && ui.vendor.options.length > 0) {
      ui.vendor.selectedIndex = 0;
    }
    ui.model.replaceChildren(
      modelPicker({
        key: "apply-model",
        label: "Model that applies the fixes",
        id: stored,
        catalogue: deps.catalogue(),
        onChange: (id) => {
          const live = deps.settings();
          live.apply_model = id;
          live.apply_effort = allowedEffort(id, live.apply_effort);
          deps.refresh();
          drawEffort();
          board.draw();
          setButtonState();
        },
      }),
    );
    drawEffort();
    ui.push.checked = deps.settings().push_after_apply;
    drawTag();
    setButtonState();
    fixes.refresh(deps.promptRepo());
    if (focusKey) {
      const selector = `[data-focus-key="${CSS.escape(focusKey)}"]`;
      (
        ui.model.querySelector<HTMLElement>(selector) ??
        ui.effort.querySelector<HTMLElement>(selector)
      )?.focus();
    }
  };

  /**
   * Tagging is only offered when pushing is.
   *
   * A release is cut from commits on the remote, so with pushing off the box
   * could never do anything — and a checkbox that is ticked but inert is how
   * someone comes to believe a release went out. Disabled rather than hidden,
   * so the dependency is visible instead of the control vanishing.
   */
  const drawTag = (): void => {
    const live = deps.settings();
    const publishing = live.push_after_apply;
    if (!publishing) live.tag_release_after_push = false;
    ui.tag.disabled = !publishing;
    ui.tag.checked = live.tag_release_after_push;
    ui.tag.title = publishing
      ? "Tag the pushed commits so the repository's CI builds a release."
      : "Turn on pushing first — a release is built from commits on the remote.";
  };

  /** Which levels apply depends on the model, so this is redrawn on its own. */
  const drawEffort = (): void => {
    ui.effort.replaceChildren(
      effortPicker({
        key: "apply-effort",
        label: "Effort for the model that applies the fixes",
        id: deps.settings().apply_model,
        effort: deps.settings().apply_effort,
        catalogue: deps.catalogue(),
        onChange: (effort) => {
          deps.settings().apply_effort = effort;
          deps.refresh();
          board.draw();
          setButtonState();
        },
      }),
    );
  };

  /** The stored effort, or nothing when this model does not take it. */
  const allowedEffort = (id: string, effort: string): string => {
    const { vendor, model } = splitId(id);
    const menu = deps.catalogue()[vendor];
    const levels = menu?.efforts.length
      ? menu.efforts
      : (menu?.efforts_by_model[model] ?? []);
    return levels.includes(effort) ? effort : "";
  };

  /**
   * The button is offered only when it would do something, and says which thing.
   *
   * No model chosen and it has nothing to run; a review in flight and it would
   * edit the code that review is reading. Both are refused by Rust as well —
   * this only saves the click. Its label switches to Resume when a previous fix
   * run left defects finished, because pressing Apply after a run that died on
   * a usage limit is otherwise indistinguishable from paying for all of them
   * again.
   */
  const setButtonState = (): void => {
    board.refresh(completionEventsReady);
    const live = deps.settings();
    const chosen = live.apply_model.trim() !== "";
    const validEffort = effortIsValid(
      live.apply_model,
      live.apply_effort,
      deps.catalogue(),
    );
    const cliPresent = vendorCliPresent(live.apply_model, deps.catalogue());
    ui.button.disabled =
      !completionEventsReady ||
      activeApplies.has(deps.promptRepo()) ||
      deps.busy() ||
      !chosen ||
      !validEffort ||
      !cliPresent;
    const resuming = fixes.alreadyFixed(deps.promptRepo());
    ui.button.textContent =
      resuming > 0 ? `Resume fixes (${resuming} done)` : "Apply fixes";
    ui.button.title = !completionEventsReady
      ? "Applying is unavailable until its result listener is ready."
      : !chosen
        ? "Choose a provider and model first."
        : !cliPresent
          ? "This provider's CLI is not installed on this machine."
          : validEffort
            ? resuming > 0
              ? `${resuming} defects are already fixed and committed. This continues at the ones still outstanding rather than starting over.`
              : "Run the fix prompt against this repository, editing files in place."
            : "This model does not accept the stored effort; choose Default first.";
  };

  ui.vendor.addEventListener("change", () => {
    deps.settings().apply_model = joinId(ui.vendor.value as Vendor, "");
    deps.settings().apply_effort = "";
    deps.refresh();
    draw();
  });

  ui.push.addEventListener("change", () => {
    deps.settings().push_after_apply = ui.push.checked;
    drawTag();
    deps.refresh();
  });

  ui.tag.addEventListener("change", () => {
    deps.settings().tag_release_after_push = ui.tag.checked;
    deps.refresh();
  });

  ui.button.addEventListener("click", () => {
    if (activeApplies.has(deps.promptRepo())) return;
    const repo = deps.promptRepo().trim();
    if (repo === "") {
      deps.setStatus(
        "The displayed result has no repository target; run the review again",
        "error",
      );
      return;
    }
    const confirmed = { ...deps.settings() };
    void confirmDialog(applyConfirmation(repo, confirmed)).then((yes) => {
      if (!yes) return;
      start(
        {
          ...deps,
          settings: () => confirmed,
          refresh: () => {
            deps.refresh();
            setButtonState();
          },
        },
        repo,
      );
    });
  });

  void listen<{
    repo: string;
    model: string;
    ok: boolean;
    /** Whether Stop was pressed, as opposed to the provider failing. */
    cancelled: boolean;
    text: string;
    changed?: string[];
  }>("apply-finished", (event) => {
    const { repo, model } = event.payload;
    activeApplies.delete(repo);
    recordApply(
      repo,
      model,
      event.payload.text,
      event.payload.cancelled
        ? "Stopped"
        : applyStatus(event.payload.ok, event.payload.changed?.length ?? 0),
    );
    if (repo === deps.promptRepo()) append(ui.output, event.payload.text);
    const changed = event.payload.changed?.length ?? 0;
    const failed = !event.payload.ok && !event.payload.cancelled;
    if (repo === deps.promptRepo())
      deps.setStatus(
        event.payload.cancelled
          ? "Applying the fixes was stopped"
          : applyStatus(event.payload.ok, changed),
        failed ? "error" : "",
      );
    if (document.activeElement === ui.stop) deps.focusStatus();
    // Whatever the outcome, the journal on disk has changed: a finished run
    // discarded it, a stopped or failed one added to it. Read it again rather
    // than infer, so the button's promise matches what Rust would actually do.
    fixes.refresh(repo);
    deps.refresh();
    draw();
  }).then(
    () => {
      completionEventsReady = true;
      ui.listenerError.textContent = "";
      ui.listenerError.classList.add("hidden");
      setButtonState();
    },
    (error: unknown) => {
      const message = `Cannot hear the result of applying: ${String(error)}`;
      ui.listenerError.textContent = message;
      ui.listenerError.classList.remove("hidden");
      deps.setStatus(message, "error");
      setButtonState();
    },
  );

  document.addEventListener("repository-report-shown", () => {
    const log = applyLog(deps.promptRepo());
    if (log) append(ui.output, log);
    draw();
  });
  draw();
  return { redraw: draw, refreshButton: setButtonState };
}

function start(deps: ApplyDeps, repo: string): void {
  if (activeApplies.has(repo) || deps.busy()) return;
  activeApplies.add(repo);
  deps.setStatus("Applying the fixes — this edits your repository", "running");
  if (document.activeElement === deps.ui.button) deps.focusStatus();
  deps.ui.button.disabled = true;
  deps.ui.stop.disabled = false;
  append(deps.ui.output, "Applying the fixes…");
  recordApply(
    repo,
    deps.settings().apply_model,
    "Applying the fixes…",
    "Running or waiting for this provider",
  );
  deps.refresh();
  const settings = settingsForApply(deps.settings(), repo);
  invoke("apply_fixes", { settings }).catch((error: unknown) => {
    activeApplies.delete(repo);
    recordApply(repo, settings.apply_model, String(error), "Failed to start");
    if (repo === deps.promptRepo()) {
      deps.setStatus(String(error), "error");
      append(deps.ui.output, String(error));
    }
    deps.refresh();
  });
}

/**
 * Add to the pane rather than replace it.
 *
 * What the review found is what the applied changes have to be read against,
 * and this is an `aria-live` region: replacing the whole text makes a screen
 * reader announce the entire report again to deliver one new line.
 */
function append(output: HTMLPreElement, text: string): void {
  const separator = output.textContent === "" ? "" : NEWLINE + NEWLINE;
  output.appendChild(document.createTextNode(separator + text));
  output.scrollTop = output.scrollHeight;
}
