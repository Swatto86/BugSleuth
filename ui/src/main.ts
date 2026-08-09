/**
 * Wiring: DOM in, commands out.
 *
 * The rules live in model.ts; this file only reflects state into elements and
 * sends what the user asked for to Rust.
 */

import { invoke } from "@tauri-apps/api/core";

import {
  LANE_TITLES,
  type Lane,
  type Settings,
  batchCount,
  canRun,
  isShippedConfiguration,
  preset,
  uncoveredLanes,
  unitCount,
} from "./model";
import { bindGuardedActions } from "./actions";
import { bindControls } from "./controls";
import { matrixHandlers } from "./matrix";
import { wireUpdate } from "./update";
import { savingSettings } from "./persist";
import { listOf } from "./format";
import {
  type RunDeps,
  currentFixPrompt,
  isRunning,
  listenForRunEvents,
} from "./run";
import { bindApply, isApplying } from "./apply";
import { ui } from "./elements";
import {
  type Catalogue,
  type VendorModels,
  type VendorStatus,
  vendorPills,
  matrixRows,
} from "./view";

/**
 * Which model re-grades severities. Cheapest available: the pass compares
 * summaries against each other, it does not review code again.
 */
const TRIAGE_MODEL = "haiku";

let settings: Settings = {
  repo: "",
  scope: "",
  models: preset("balanced"),
  theme: "system",
  reuse_completed: true,
  provider_concurrency: 3,
  triage_model: TRIAGE_MODEL,
  apply_model: "",
  apply_effort: "",
  push_after_apply: false,
  tag_release_after_push: false,
};
/**
 * What the model and effort dropdowns offer, keyed by vendor.
 *
 * Starts empty so the table renders immediately with typed-model fallbacks,
 * and is filled in when the vendors answer. Waiting for it would make the whole
 * window wait on three CLIs.
 */
let catalogue: Catalogue = {};

// ── Theme ───────────────────────────────────────────────────────────────────

/**
 * `system` removes the attribute entirely rather than resolving it here, so the
 * CSS media query takes over and the app follows the OS live — resolving it in
 * JS would freeze the choice at whatever it was when the app started.
 */
function applyTheme(theme: Settings["theme"]): void {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
}

// ── Rendering ───────────────────────────────────────────────────────────────

/**
 * Surface uncovered lanes prominently, and mark the column heads.
 *
 * This is the UI's most important job. The report will say "not swept" either
 * way, but by then the run is paid for; here it is still free to fix.
 */
function renderCoverage(): void {
  const uncovered = uncoveredLanes(settings.models);

  for (const head of document.querySelectorAll<HTMLElement>("th.lane-cell")) {
    const lane = head.dataset["lane"] ?? "";
    head.classList.toggle("uncovered", uncovered.includes(lane as Lane));
  }

  if (uncovered.length === 0) {
    ui.uncovered.classList.add("hidden");
    ui.uncovered.textContent = "";
    return;
  }
  const names = uncovered.map((lane) => LANE_TITLES[lane]);
  ui.uncovered.classList.remove("hidden");
  ui.uncovered.textContent =
    `No model covers ${listOf(names)}. ${uncovered.length === 1 ? "That lane" : "Those lanes"} ` +
    `will be reported as NOT SWEPT — nothing will be looked for there, which is not ` +
    `the same as nothing being wrong.`;
}

function renderPlanSummary(): void {
  const units = unitCount(settings.models);
  const rounds = batchCount(settings.models, settings.provider_concurrency);
  ui.planSummary.textContent =
    units === 0
      ? ""
      : `${units} sweep${units === 1 ? "" : "s"} · ${rounds} round${rounds === 1 ? "" : "s"}`;
  // Not while fixes are being applied: a sweep would be reading the tree the
  // apply is rewriting, and the report would describe code that no longer
  // exists. Rust refuses it too — this only saves the click.
  ui.run.disabled = isRunning() || isApplying() || !canRun(settings);
  // Same reason, and the same defect if it is left out: Rust refuses to delete
  // while it is writing there, so an offered button would put up a dialog about
  // losing paid-for sweeps and then answer it with an error.
  ui.clearSaved.disabled = isRunning() || isApplying();
  // Offered only while there is something to stop, so it is never a button
  // that does nothing.
  ui.stop.classList.toggle("hidden", !isRunning());
}

/**
 * Rebuild the model table, putting keyboard focus back where it was.
 *
 * Toggling a lane replaces every element in the table, so focus fell to
 * `<body>` — on the app's busiest control, a keyboard user was thrown back to
 * the top of the page on every single tick. There is no workaround for that
 * except tabbing all the way in again, each time.
 *
 * The identity is a `data-focus-key` the row builder writes. Restoring by
 * position would put focus on a different lane the moment a row is removed.
 */
function render(): void {
  const focused = document.activeElement;
  const key =
    focused instanceof HTMLElement ? focused.dataset["focusKey"] : undefined;

  renderRows();

  if (key === undefined) return;
  const restored = ui.matrixBody.querySelector<HTMLElement>(
    `[data-focus-key="${CSS.escape(key)}"]`,
  );
  if (restored) {
    restored.focus();
    return;
  }

  // Removing the focused final row deletes its focus key. Continue at the new
  // final row, or at Add model when the matrix is now empty — otherwise focus
  // falls to <body> and a keyboard user restarts from the top of the page.
  if (key.startsWith("remove-")) {
    const lastRemove = ui.matrixBody.querySelector<HTMLElement>(
      `[data-focus-key="remove-${settings.models.length - 1}"]`,
    );
    (lastRemove ?? ui.addModel).focus();
  }
}

const rowHandlers = matrixHandlers({
  settings: () => settings,
  render,
  refresh,
});

function renderRows(): void {
  ui.matrixBody.replaceChildren(
    ...matrixRows(settings.models, catalogue, rowHandlers),
  );
  refresh();
}

// A render triggered by boot or a catalogue load must not write settings back:
// if the saved file was unreadable the app is showing defaults, and persisting
// them would overwrite the recoverable file. Only a user edit persists.
let suppressPersistence = false;

/** Render without letting the render persist — for boot and catalogue loads. */
function renderWithoutPersisting(): void {
  suppressPersistence = true;
  try {
    render();
  } finally {
    suppressPersistence = false;
  }
}

/** Re-render everything that depends on state but not on the table's identity. */
function refresh(): void {
  renderCoverage();
  renderPlanSummary();
  if (!suppressPersistence) persist();
}

function setStatus(text: string, kind: "" | "running" | "error" = ""): void {
  ui.status.textContent = text;
  ui.status.className = `status ${kind}`.trim();
  ui.spinner.classList.toggle("hidden", kind !== "running");
}

function focusStatus(): void {
  ui.status.focus();
}

/**
 * The settings-save error has its own region, separate from the status bar, so
 * a run's transient messages can never erase it and it survives until a later
 * save succeeds.
 */
function setSettingsError(text: string): void {
  ui.settingsError.textContent = text;
  ui.settingsError.classList.toggle("hidden", text === "");
}

/** Save settings, reporting a failure the user would otherwise never see. */
const persist = savingSettings({
  settings: () => settings,
  setError: setSettingsError,
});

const runDeps = (): RunDeps => ({
  output: ui.output,
  stop: ui.stop,
  findings: ui.findings,
  copyPrompt: ui.copyPrompt,
  promptPath: ui.promptPath,
  applyPanel: ui.applyPanel,
  setStatus,
  focusStatus,
  renderPlanSummary,
  settings: () => settings,
});

// ── Boot ────────────────────────────────────────────────────────────────────

/** Redraw the apply panel. Assigned when it is bound; the vendor menus arrive
 * later, and its model suggestions come from them. */
let redrawApply: () => void = () => {};

function bind(): void {
  redrawApply = bindApply({
    ui: {
      vendor: ui.applyVendor,
      model: ui.applyModel,
      effort: ui.applyEffort,
      button: ui.applyFixes,
      push: ui.pushAfterApply,
      tag: ui.tagReleaseAfterPush,
      output: ui.output,
    },
    settings: () => settings,
    catalogue: () => catalogue,
    busy: isRunning,
    refresh,
    setStatus,
    focusStatus,
  });

  bindControls({
    ui,
    settings: () => settings,
    triageModel: TRIAGE_MODEL,
    applyTheme,
    refresh,
    render,
    setStatus,
    focusStatus,
    fixPrompt: currentFixPrompt,
  });

  wireUpdate({
    button: ui.checkUpdate,
    setStatus,
    focusStatus,
    // Installing restarts the process, so neither reviews nor repository edits
    // may be in flight.
    busy: () => isRunning() || isApplying(),
  });

  bindGuardedActions({
    ui,
    settings: () => settings,
    setSettings: (models) => {
      settings.models = models;
    },
    render,
    setStatus,
    focusStatus,
    runDeps,
    isShippedConfiguration,
  });
}

/** Fill the model and effort menus, then redraw the table with them. */
async function loadCatalogue(): Promise<string> {
  try {
    const vendors = await invoke<VendorModels[]>("available_models");
    catalogue = Object.fromEntries(vendors.map((v) => [v.vendor, v]));
    renderWithoutPersisting();
    // The apply panel has its own model box, outside the table, so a redraw of
    // the table alone would leave it offering no suggestions for the session.
    redrawApply();
    return "";
  } catch (error) {
    // Not swallowed: when the catalogue cannot load, every effort select stays
    // disabled and the model menus stay empty for the whole session with no
    // retry, so the user has to be told. The table still works with typed ids.
    return `Could not load the model lists: ${String(error)}`;
  }
}

async function boot(): Promise<void> {
  bind();

  let loadFailed = "";
  try {
    settings = { ...settings, ...(await invoke<Settings>("load_settings")) };
  } catch (error) {
    // A first launch genuinely has no settings, and Rust answers with defaults
    // rather than an error for that case — so reaching here means the call
    // itself failed, and starting from defaults would quietly discard whatever
    // was saved. Say so instead.
    loadFailed = String(error);
  }

  applyTheme(settings.theme);
  ui.theme.value = settings.theme;
  ui.repo.value = settings.repo;
  ui.scope.value = settings.scope;
  ui.providerConcurrency.value = String(settings.provider_concurrency);
  ui.reuseCompleted.checked = settings.reuse_completed;
  ui.triageSeverities.checked = settings.triage_model.trim() !== "";
  renderWithoutPersisting();
  // The panel was bound before the stored settings arrived, so it is showing
  // the defaults until told otherwise.
  redrawApply();

  // Reveal as soon as the shell is painted and themed. Deliberately before the
  // event subscriptions below: if one of those ever fails, the window must
  // still appear. An app that starts invisible with no way to recover is the
  // exact failure the UX lane exists to catch.
  // invoke-may-fail-silently: this only asks Rust to reveal the window, and
  // Rust reveals it on a timer anyway precisely so a failure here cannot leave
  // an invisible app. Reporting it would need the window this call is for.
  void invoke("frontend_ready");

  await listenForRunEvents(runDeps());

  // Filled before the status settles, so a failure here is part of what
  // "Ready" means: the effort controls stay disabled and the model menus stay
  // empty for the whole session if the catalogue cannot load. The window is
  // already revealed by this point, so awaiting costs no visible delay.
  setStatus("Checking providers…", "running");
  const catalogueError = await loadCatalogue();
  try {
    ui.vendors.replaceChildren(
      ...vendorPills(await invoke<VendorStatus[]>("preflight")),
    );
    setStatus(
      catalogueError
        ? catalogueError
        : loadFailed
          ? `Saved settings could not be read: ${loadFailed}`
          : "Ready",
      catalogueError ? "error" : loadFailed ? "error" : "",
    );
  } catch (error) {
    setStatus(String(error), "error");
  }
}

// A failure anywhere in boot must not leave an invisible window behind. Rust
// also reveals the window on a timer as a second line of defence.
void boot().catch((error: unknown) => {
  // invoke-may-fail-silently: last-resort reveal on a failed boot. If this
  // fails too, Rust's timer is the remaining line of defence and there is no
  // surface left to report on.
  void invoke("frontend_ready");
  setStatus(`Startup problem: ${String(error)}`, "error");
});
