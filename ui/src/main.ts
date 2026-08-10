/** Wiring: DOM in, commands out. */

import { invoke } from "@tauri-apps/api/core";

import {
  type Settings,
  batchCount,
  canRun,
  isShippedConfiguration,
  preset,
  supportsAgents,
  usesUltracode,
  unitCount,
} from "./model";
import { bindGuardedActions, isClearing } from "./actions";
import { bindControls } from "./controls";
import { matrixHandlers } from "./matrix";
import { isUpdating, wireUpdate } from "./update";
import { savingSettings } from "./persist";
import {
  type RunDeps,
  currentFixPrompt,
  currentFixPromptRepo,
  currentRunReport,
  isRunning,
  listenForRunEvents,
} from "./run";
import { type ApplyBinding, bindApply, isApplying } from "./apply";
import { ui } from "./elements";
import { renderCoverage, withRestoredFocus } from "./plan-view";
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

function renderPlanSummary(): void {
  const units = unitCount(settings.models);
  const rounds = batchCount(settings.models);
  const summary =
    units === 0
      ? ""
      : `${units} sweep${units === 1 ? "" : "s"} · ${rounds} round${rounds === 1 ? "" : "s"}`;
  if (ui.planSummary.textContent !== summary)
    ui.planSummary.textContent = summary;
  const busy = isRunning() || isApplying() || isClearing() || isUpdating();
  ui.run.disabled = busy || !canRun(settings, catalogue);
  ui.clearSaved.disabled = busy;
  ui.stop.classList.toggle("hidden", !isRunning() && !isApplying());
}

function render(): void {
  withRestoredFocus(settings.models.length, renderRows);
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
  renderCoverage(settings.models);
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
const settingsSaver = savingSettings({
  settings: () => settings,
  setError: setSettingsError,
});
const persist = (): void => settingsSaver.schedule();

const runDeps = (): RunDeps => ({
  output: ui.output,
  stop: ui.stop,
  findings: ui.findings,
  copyReport: ui.copyReport,
  copyPrompt: ui.copyPrompt,
  promptPath: ui.promptPath,
  applyPanel: ui.applyPanel,
  setStatus,
  focusStatus,
  renderPlanSummary,
  settings: () => settings,
});

// ── Boot ────────────────────────────────────────────────────────────────────

let applyBinding: ApplyBinding = {
  redraw: () => {},
  refreshButton: () => {},
};

function bind(): void {
  applyBinding = bindApply({
    ui: {
      vendor: ui.applyVendor,
      model: ui.applyModel,
      effort: ui.applyEffort,
      button: ui.applyFixes,
      stop: ui.stop,
      push: ui.pushAfterApply,
      tag: ui.tagReleaseAfterPush,
      output: ui.output,
    },
    settings: () => settings,
    promptRepo: currentFixPromptRepo,
    catalogue: () => catalogue,
    busy: () => isRunning() || isClearing() || isUpdating(),
    refresh,
    setStatus,
    focusStatus,
  });

  const activityChanged = (): void => {
    renderPlanSummary();
    applyBinding.refreshButton();
  };

  bindControls({
    ui,
    settings: () => settings,
    triageModel: TRIAGE_MODEL,
    applyTheme,
    refresh,
    render,
    setStatus,
    focusStatus,
    report: currentRunReport,
    fixPrompt: currentFixPrompt,
  });

  wireUpdate({
    button: ui.checkUpdate,
    setStatus,
    focusStatus,
    busy: () => isRunning() || isApplying() || isClearing(),
    activityChanged,
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
    updating: isUpdating,
    activityChanged,
    flushSettings: () => settingsSaver.flush(),
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
    applyBinding.redraw();
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
    settingsSaver.allowWrites();
  } catch (error) {
    // A first launch genuinely has no settings, and Rust answers with defaults
    // rather than an error for that case — so reaching here means the call
    // itself failed, and starting from defaults would quietly discard whatever
    // was saved. Say so instead — and keep writes closed for the session, or
    // the first edit replaces the unreadable file with those defaults before
    // the warning below has even been shown.
    loadFailed = String(error);
    settingsSaver.blockWrites(
      `Saved settings could not be read: ${loadFailed}`,
    );
  }
  settings.models = settings.models.map((model) => {
    const useAgents = supportsAgents(model.id) && (model.use_agents ?? false);
    return {
      ...model,
      effort: useAgents && usesUltracode(model.id) ? "" : model.effort,
      use_agents: useAgents,
    };
  });

  applyTheme(settings.theme);
  ui.theme.value = settings.theme;
  ui.repo.value = settings.repo;
  ui.scope.value = settings.scope;
  ui.reuseCompleted.checked = settings.reuse_completed;
  ui.triageSeverities.checked = settings.triage_model.trim() !== "";
  renderWithoutPersisting();
  applyBinding.redraw();

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
//
// Check sign-in is enabled only here, after the last startup writer of the
// vendor pills and the status has settled. Enabled from the first paint, a
// click during startup could finish first and then be replaced by boot's
// weaker executable-only preflight result. A failed boot reaches the finalizer
// too, and in that case no later startup writer remains.
void boot()
  .catch((error: unknown) => {
    // invoke-may-fail-silently: last-resort reveal on a failed boot. If this
    // fails too, Rust's timer is the remaining line of defence and there is no
    // surface left to report on.
    void invoke("frontend_ready");
    setStatus(`Startup problem: ${String(error)}`, "error");
  })
  .finally(() => {
    ui.checkSignin.disabled = false;
  });
