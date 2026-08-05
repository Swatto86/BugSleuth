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
  type Preset,
  type Settings,
  batchCount,
  canRun,
  preset,
  toggleLane,
  uncoveredLanes,
  unitCount,
} from "./model";
import { bindGuardedActions } from "./actions";
import { bindControls } from "./controls";
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

/**
 * Whether a row is still exactly as some preset shipped it.
 *
 * Used to decide whether replacing the matrix would destroy anything: asking
 * for confirmation when there is nothing to lose is how people learn to click
 * through confirmations without reading them.
 */
function isShipped(model: Settings["models"][number]): boolean {
  return (["cheap", "balanced", "deep"] as Preset[]).some((name) =>
    preset(name).some(
      (shipped) =>
        shipped.id === model.id &&
        shipped.effort === model.effort &&
        (shipped.passes ?? 1) === (model.passes ?? 1) &&
        shipped.lanes.length === model.lanes.length &&
        shipped.lanes.every((lane) => model.lanes.includes(lane)),
    ),
  );
}

let settings: Settings = {
  repo: "",
  scope: "",
  models: preset("balanced"),
  theme: "system",
  prove_top: 0,
  test_command: "",
  reuse_completed: true,
  triage_model: TRIAGE_MODEL,
  apply_model: "",
  apply_effort: "",
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
  const rounds = batchCount(settings.models);
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
  // Nothing to restore to when the row itself was just removed. Leaving focus
  // where the browser put it beats guessing at a neighbour.
  restored?.focus();
}

function renderRows(): void {
  ui.matrixBody.replaceChildren(
    ...matrixRows(settings.models, catalogue, {
      onRename: (index, id) => {
        const existing = settings.models[index];
        if (existing) settings.models[index] = { ...existing, id };
        refresh();
      },
      onVendor: (index, id) => {
        const existing = settings.models[index];
        // The effort goes with the old vendor: the levels differ between them,
        // and carrying one over would send a value the new CLI may reject.
        if (existing) settings.models[index] = { ...existing, id, effort: "" };
        render();
      },
      onPasses: (index, passes) => {
        const existing = settings.models[index];
        if (existing) settings.models[index] = { ...existing, passes };
        refresh();
      },
      onEffort: (index, effort) => {
        const existing = settings.models[index];
        if (existing) settings.models[index] = { ...existing, effort };
        refresh();
      },
      onToggle: (index, lane, on) => {
        settings.models = toggleLane(settings.models, index, lane, on);
        render();
      },
      onRemove: (index) => {
        settings.models = settings.models.filter((_, i) => i !== index);
        render();
      },
    }),
  );
  refresh();
}

/** Re-render everything that depends on state but not on the table's identity. */
function refresh(): void {
  renderCoverage();
  renderPlanSummary();
  persist();
}

function setStatus(text: string, kind: "" | "running" | "error" = ""): void {
  ui.status.textContent = text;
  ui.status.className = `status ${kind}`.trim();
  ui.spinner.classList.toggle("hidden", kind !== "running");
}

/** Save settings, reporting a failure the user would otherwise never see. */
const persist = savingSettings({
  settings: () => settings,
  setStatus,
  quiet: () => isRunning(),
});

const runDeps = (): RunDeps => ({
  output: ui.output,
  stop: ui.stop,
  findings: ui.findings,
  copyPrompt: ui.copyPrompt,
  promptPath: ui.promptPath,
  applyPanel: ui.applyPanel,
  setStatus,
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
      output: ui.output,
    },
    settings: () => settings,
    catalogue: () => catalogue,
    busy: isRunning,
    refresh,
    setStatus,
  });

  bindControls({
    ui,
    settings: () => settings,
    triageModel: TRIAGE_MODEL,
    applyTheme,
    refresh,
    render,
    setStatus,
    fixPrompt: currentFixPrompt,
  });

  wireUpdate({
    button: ui.checkUpdate,
    setStatus,
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
    runDeps,
    isShipped,
  });
}

/** Fill the model and effort menus, then redraw the table with them. */
async function loadCatalogue(): Promise<void> {
  try {
    const vendors = await invoke<VendorModels[]>("available_models");
    catalogue = Object.fromEntries(vendors.map((v) => [v.vendor, v]));
    render();
    // The apply panel has its own model box, outside the table, so a redraw of
    // the table alone would leave it offering no suggestions for the session.
    redrawApply();
  } catch {
    // The table already works with typed model ids, so a catalogue that cannot
    // be fetched costs convenience rather than capability.
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
  // A stored zero means proving is off; the switch says so and the fields that
  // only matter when it is on are hidden. Showing "0" in a number box was how
  // the first person to open this could not tell whether it was on.
  ui.proveEnabled.checked = settings.prove_top > 0;
  ui.proveSettings.classList.toggle("hidden", settings.prove_top === 0);
  ui.proveTop.value = String(Math.max(1, settings.prove_top));
  ui.testCommand.value = settings.test_command;
  ui.reuseCompleted.checked = settings.reuse_completed;
  ui.triageSeverities.checked = settings.triage_model.trim() !== "";
  render();
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

  // After the reveal on purpose: filling the menus asks Kilo for its catalogue,
  // and the window must not wait on a CLI to appear.
  void loadCatalogue();

  setStatus("Checking providers…", "running");
  try {
    ui.vendors.replaceChildren(
      ...vendorPills(await invoke<VendorStatus[]>("preflight")),
    );
    setStatus(
      loadFailed ? `Saved settings could not be read: ${loadFailed}` : "Ready",
      loadFailed ? "error" : "",
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
