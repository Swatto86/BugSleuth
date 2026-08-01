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
import { listOf } from "./format";
import { type RunDeps, currentFixPrompt, isRunning, listenForRunEvents, startRun } from "./run";
import {
  type Catalogue,
  type VendorModels,
  type VendorStatus,
  vendorPills,
  matrixRows,
} from "./view";



const el = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing element #${id}`);
  return found as T;
};

const ui = {
  theme: el<HTMLSelectElement>("theme"),
  vendors: el<HTMLDivElement>("vendors"),
  repo: el<HTMLInputElement>("repo"),
  scope: el<HTMLInputElement>("scope"),
  browse: el<HTMLButtonElement>("browse"),
  matrixBody: el<HTMLTableSectionElement>("matrix-body"),
  addModel: el<HTMLButtonElement>("add-model"),
  uncovered: el<HTMLDivElement>("uncovered-warning"),
  proveTop: el<HTMLInputElement>("prove-top"),
  testCommand: el<HTMLInputElement>("test-command"),
  reuseCompleted: el<HTMLInputElement>("reuse-completed"),
  triageSeverities: el<HTMLInputElement>("triage-severities"),
  output: el<HTMLPreElement>("output"),
  findings: el<HTMLDivElement>("findings"),
  status: el<HTMLSpanElement>("status"),
  spinner: el<HTMLSpanElement>("spinner"),
  planSummary: el<HTMLSpanElement>("plan-summary"),
  run: el<HTMLButtonElement>("run"),
  quit: el<HTMLButtonElement>("quit"),
  copyPrompt: el<HTMLButtonElement>("copy-prompt"),
  promptPath: el<HTMLParagraphElement>("prompt-path"),
};

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
  prove_top: 0,
  test_command: "",
  reuse_completed: true,
  triage_model: TRIAGE_MODEL,
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
    units === 0 ? "" : `${units} sweep${units === 1 ? "" : "s"} · ${rounds} round${rounds === 1 ? "" : "s"}`;
  ui.run.disabled = isRunning() || !canRun(settings);
}

function render(): void {
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
  void persist();
}

function setStatus(text: string, kind: "" | "running" | "error" = ""): void {
  ui.status.textContent = text;
  ui.status.className = `status ${kind}`.trim();
  ui.spinner.classList.toggle("hidden", kind !== "running");
}

// ── Commands ────────────────────────────────────────────────────────────────

let saveTimer: number | undefined;
/** Whether the last save attempt failed, so recovery can be reported too. */
let saveFailed = false;

/**
 * Persist settings, coalesced so typing does not write on every keystroke.
 *
 * A failure here used to be swallowed on the grounds that losing a preference
 * is not worth interrupting anyone over. That reasoning was wrong in a way that
 * took hours to unpick: saving failed for a whole session, the app looked
 * perfectly normal, and every configuration made in it was lost on restart. The
 * cost of a wrong preference is small; the cost of not knowing your settings
 * are not being kept is not.
 *
 * So it is reported in the status bar — not a dialog, because it must not
 * interrupt a run in progress — and only while nothing else is being said.
 */
function persist(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    void invoke("save_settings", { settings })
      .then(() => {
        if (saveFailed) {
          saveFailed = false;
          if (!isRunning()) setStatus("Ready");
        }
      })
      .catch((error: unknown) => {
        saveFailed = true;
        if (!isRunning()) setStatus(`Settings are not being saved: ${String(error)}`, "error");
      });
  }, 400);
}


const runDeps = (): RunDeps => ({
  output: ui.output,
  findings: ui.findings,
  copyPrompt: ui.copyPrompt,
  promptPath: ui.promptPath,
  setStatus,
  renderPlanSummary,
  settings: () => settings,
});

// ── Boot ────────────────────────────────────────────────────────────────────

function bind(): void {
  ui.theme.addEventListener("change", () => {
    settings.theme = ui.theme.value as Settings["theme"];
    applyTheme(settings.theme);
    refresh();
  });

  ui.repo.addEventListener("input", () => {
    settings.repo = ui.repo.value;
    refresh();
  });
  ui.scope.addEventListener("input", () => {
    settings.scope = ui.scope.value;
    refresh();
  });
  ui.proveTop.addEventListener("input", () => {
    settings.prove_top = Math.max(0, Number(ui.proveTop.value) || 0);
    refresh();
  });
  ui.testCommand.addEventListener("input", () => {
    settings.test_command = ui.testCommand.value;
    refresh();
  });
  ui.reuseCompleted.addEventListener("change", () => {
    settings.reuse_completed = ui.reuseCompleted.checked;
    refresh();
  });
  ui.triageSeverities.addEventListener("change", () => {
    // Off is an empty model rather than a separate flag, so there is one thing
    // to read on the Rust side and no way for the two to disagree.
    settings.triage_model = ui.triageSeverities.checked ? TRIAGE_MODEL : "";
    refresh();
  });

  ui.browse.addEventListener("click", async () => {
    const picked = await invoke<string | null>("pick_directory");
    if (picked) {
      settings.repo = picked;
      ui.repo.value = picked;
      refresh();
    }
  });

  ui.addModel.addEventListener("click", () => {
    settings.models = [
      ...settings.models,
      { id: "", lanes: [], effort: "", passes: 1 },
    ];
    render();
  });

  for (const name of ["cheap", "balanced", "deep"] as Preset[]) {
    el<HTMLButtonElement>(`preset-${name}`).addEventListener("click", () => {
      settings.models = preset(name);
      render();
    });
  }

  ui.run.addEventListener("click", () => void startRun(runDeps()));

  // Closing the window only hides it. This is the reachable, keyboard-navigable
  // way to actually exit, so the tray is not the single point of failure.
  ui.quit.addEventListener("click", () => void invoke("quit"));

  ui.copyPrompt.addEventListener("click", () => {
    void navigator.clipboard.writeText(currentFixPrompt()).then(
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

/** Fill the model and effort menus, then redraw the table with them. */
async function loadCatalogue(): Promise<void> {
  try {
    const vendors = await invoke<VendorModels[]>("available_models");
    catalogue = Object.fromEntries(vendors.map((v) => [v.vendor, v]));
    render();
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
  ui.proveTop.value = String(settings.prove_top);
  ui.testCommand.value = settings.test_command;
  ui.reuseCompleted.checked = settings.reuse_completed;
  ui.triageSeverities.checked = settings.triage_model.trim() !== "";
  render();

  // Reveal as soon as the shell is painted and themed. Deliberately before the
  // event subscriptions below: if one of those ever fails, the window must
  // still appear. An app that starts invisible with no way to recover is the
  // exact failure the UX lane exists to catch.
  void invoke("frontend_ready");

  await listenForRunEvents(runDeps());

  // After the reveal on purpose: filling the menus asks Kilo for its catalogue,
  // and the window must not wait on a CLI to appear.
  void loadCatalogue();

  setStatus("Checking providers…", "running");
  try {
    ui.vendors.replaceChildren(...vendorPills(await invoke<VendorStatus[]>("preflight")));
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
  void invoke("frontend_ready");
  setStatus(`Startup problem: ${String(error)}`, "error");
});
