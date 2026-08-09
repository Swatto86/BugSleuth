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

import { confirmDialog } from "./dialog";
import {
  VENDORS,
  applyStatus,
  joinId,
  settingsForApply,
  splitId,
  type Settings,
  type Vendor,
} from "./model";
import { effortPicker, modelPicker, option } from "./pickers";
import type { Catalogue } from "./view";

const NEWLINE = String.fromCharCode(10);

export interface ApplyDeps {
  ui: {
    vendor: HTMLSelectElement;
    /** Holds the model box, which is rebuilt when the provider changes. */
    model: HTMLDivElement;
    /** Holds the effort control, rebuilt whenever the model changes. */
    effort: HTMLDivElement;
    button: HTMLButtonElement;
    /** Opt-in to publishing what the apply commits. */
    push: HTMLInputElement;
    /** Opt-in to tagging what the push published, so CI releases it. */
    tag: HTMLInputElement;
    output: HTMLPreElement;
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

let applying = false;
export const isApplying = (): boolean => applying;

/** Wire the panel up, and hand back the redraw the caller needs when the vendor
 * menus finally arrive. */
export function bindApply(deps: ApplyDeps): () => void {
  const { ui } = deps;

  for (const name of VENDORS) {
    ui.vendor.append(option(name, name, false));
  }

  /** Draw the provider, model and effort controls from the stored settings. */
  const draw = (): void => {
    // Read through `deps.settings()` every time, never captured into a handler.
    // The window replaces its settings object once, when the stored ones load,
    // and a closure holding the old one would write to an object nothing reads.
    const stored = deps.settings().apply_model;
    // `splitId("")` answers Claude, which is also what typing a bare model name
    // into the box would mean — so that is what the provider control has to
    // show. Setting it to "" left the control blank with no option matching,
    // and choosing "claude" from the list made it go blank again: the stored
    // spec for a Claude model with no name *is* the empty string.
    ui.vendor.value = splitId(stored).vendor;
    ui.model.replaceChildren(
      modelPicker({
        key: "apply-model",
        label: "Model that applies the fixes",
        id: stored,
        catalogue: deps.catalogue(),
        onChange: (id) => {
          const live = deps.settings();
          live.apply_model = id;
          // A model that does not accept the effort already chosen must not
          // keep it — it would be sent to the CLI and rejected. Same reset the
          // matrix does, for the same reason.
          live.apply_effort = allowedEffort(id, live.apply_effort);
          deps.refresh();
          drawEffort();
          setButtonState();
        },
      }),
    );
    drawEffort();
    ui.push.checked = deps.settings().push_after_apply;
    drawTag();
    setButtonState();
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
    // Corrected in the settings, not just in the display. A stored file can
    // say "tag but do not push" — hand-edited, or written before pushing was
    // switched off — and showing an unticked box over a stored `true` means
    // the window and the thing it sends Rust disagree about whether a release
    // is armed. Rust refuses that combination anyway; this makes the two say
    // the same thing rather than relying on the far end to be the only guard.
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
   * The button is offered only when it would do something.
   *
   * No model chosen and it has nothing to run; a review in flight and it would
   * edit the code that review is reading. Both are refused by Rust as well —
   * this only saves the click.
   */
  const setButtonState = (): void => {
    const chosen = deps.settings().apply_model.trim() !== "";
    ui.button.disabled = applying || deps.busy() || !chosen;
    ui.button.title = chosen
      ? "Run the fix prompt against this repository, editing files in place."
      : "Choose a provider and model first.";
  };

  ui.vendor.addEventListener("change", () => {
    // The model goes with the old provider: an id from one vendor means nothing
    // to another, and carrying it over would send a real-looking model id to a
    // CLI that has never heard of it. The effort goes with it for the same
    // reason — the levels differ between vendors.
    deps.settings().apply_model = joinId(ui.vendor.value as Vendor, "");
    deps.settings().apply_effort = "";
    deps.refresh();
    draw();
  });

  ui.push.addEventListener("change", () => {
    deps.settings().push_after_apply = ui.push.checked;
    // `drawTag` clears the tag setting whenever pushing is off, so switching
    // pushing off here disarms the release rather than leaving a `true` behind
    // a disabled box — which would come back the moment pushing was turned on
    // again. Redrawn before the save so what is written is what is shown.
    drawTag();
    deps.refresh();
  });

  ui.tag.addEventListener("change", () => {
    deps.settings().tag_release_after_push = ui.tag.checked;
    deps.refresh();
  });

  ui.button.addEventListener("click", () => {
    // Checked here as well as inside `start`, so a second click cannot even
    // open a second dialog on top of the first.
    if (applying) return;
    const repo = deps.promptRepo().trim();
    if (repo === "") {
      deps.setStatus(
        "The displayed result has no repository target; run the review again",
        "error",
      );
      return;
    }
    // The confirmation has to describe what is about to happen, not what
    // usually happens. Publishing is the one part of an apply that no amount
    // of reading the diff afterwards can undo, so when it is switched on the
    // dialog says so and the button that confirms it says so too — a settings
    // checkbox someone ticked days ago is not informed consent at the moment
    // the commits leave the machine.
    const publishing = deps.settings().push_after_apply;
    // Read together: tagging is stored off whenever pushing is, but the dialog
    // is the last thing between here and someone else's release pipeline, so it
    // does not take that on trust.
    const releasing = publishing && deps.settings().tag_release_after_push;
    void confirmDialog({
      title: releasing
        ? "Apply these fixes, push them, and tag a release?"
        : publishing
          ? "Apply these fixes and push them?"
          : "Apply these fixes to your code?",
      message:
        `This runs the displayed fix prompt against "${repo}" with write access, ` +
        "editing files in place and running your tests. It is refused unless " +
        "the working tree is clean, so everything it does will show up in " +
        "`git diff` and `git log` — but nothing it writes has been checked by " +
        "anyone. Read the changes before you keep them." +
        (publishing
          ? " Whatever it commits will then be pushed to this branch's " +
            "upstream. That part cannot be undone: once the commits are on " +
            "the remote, anyone watching it can fetch them, and a later " +
            "rewrite does not recall them."
          : "") +
        (releasing
          ? " The pushed commits will then be tagged with the next patch " +
            "version, which starts your CI — so this can build and publish a " +
            "release of code nobody has read yet."
          : ""),
      confirmLabel: releasing
        ? "Apply, push and tag"
        : publishing
          ? "Apply and push"
          : "Apply the fixes",
      destructive: true,
    }).then((yes) => {
      if (!yes) return;
      start(deps, repo);
    });
  });

  void listen<{ ok: boolean; text: string; changed?: string[] }>(
    "apply-finished",
    (event) => {
      applying = false;
      append(ui.output, event.payload.text);
      const changed = event.payload.changed?.length ?? 0;
      deps.setStatus(
        applyStatus(event.payload.ok, changed),
        event.payload.ok ? "" : "error",
      );
      deps.refresh();
      draw();
    },
  ).catch((error: unknown) => {
    deps.setStatus(
      `Cannot hear the result of applying: ${String(error)}`,
      "error",
    );
  });

  draw();
  // Handed back rather than exported: the panel keeps its own state, and the
  // caller only needs to say "the menus arrived, draw again".
  return draw;
}

function start(deps: ApplyDeps, repo: string): void {
  // Checked again, after the dialog. It can sit open for as long as anyone
  // likes, and two confirmed dialogs used to send two `apply_fixes` calls: the
  // second is refused by Rust, and its rejection then cleared the flag and
  // re-enabled the button while the first was still editing the repository.
  if (applying) return;
  applying = true;
  deps.setStatus("Applying the fixes — this edits your repository", "running");
  if (document.activeElement === deps.ui.button) deps.focusStatus();
  deps.ui.button.disabled = true;
  // Redrawn, not just disabled here: the Run button is disabled from the same
  // flag, and without this it stayed live for the whole apply — every press
  // rejected by Rust with an error, which reads as the app being broken rather
  // than as a button that should not have been offered.
  deps.refresh();
  append(deps.ui.output, "Applying the fixes…");
  const settings = settingsForApply(deps.settings(), repo);
  invoke("apply_fixes", { settings }).catch((error: unknown) => {
    // A rejected call means nothing started, so the flag must come back off or
    // the button stays dead for the rest of the session with nothing running.
    applying = false;
    deps.ui.button.disabled = false;
    deps.setStatus(String(error), "error");
    append(deps.ui.output, String(error));
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
