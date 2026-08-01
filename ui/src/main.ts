/**
 * Wiring: DOM in, commands out.
 *
 * The rules live in model.ts; this file only reflects state into elements and
 * sends what the user asked for to Rust.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
import { type RunEvent, describe, listOf } from "./format";
import { type VendorStatus, vendorPills, matrixRows } from "./view";



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
  output: el<HTMLPreElement>("output"),
  status: el<HTMLSpanElement>("status"),
  spinner: el<HTMLSpanElement>("spinner"),
  planSummary: el<HTMLSpanElement>("plan-summary"),
  run: el<HTMLButtonElement>("run"),
  quit: el<HTMLButtonElement>("quit"),
  billing: el<HTMLParagraphElement>("billing"),
};

let settings: Settings = {
  repo: "",
  scope: "",
  models: preset("balanced"),
  theme: "system",
  prove_top: 0,
  test_command: "",
};
let running = false;
const NEWLINE = String.fromCharCode(10);

/** Lines accumulated during a run, newest last. */
let progressLog: string[] = [];

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
  ui.run.disabled = running || !canRun(settings);
}

function render(): void {
  ui.matrixBody.replaceChildren(
    ...matrixRows(settings.models, {
      onRename: (index, id) => {
        const existing = settings.models[index];
        if (existing) settings.models[index] = { ...existing, id };
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
  void renderBilling();
  void persist();
}

/**
 * Say which account each model will spend from.
 *
 * Only Kilo can reach one model through several billing routes, and it encodes
 * which in the id — `kilo/z-ai/glm-5` spends Kilo Gateway credit while
 * `openrouter/z-ai/glm-5` spends your own OpenRouter key. Same model, different
 * bill, and nothing in the run output would tell you which. Better shown before
 * a run than worked out afterwards.
 */
async function renderBilling(): Promise<void> {
  try {
    const routes = await invoke<[string, string][]>("billing_routes", { settings });
    ui.billing.textContent = routes.length
      ? routes.map(([model, route]) => `${model} → ${route}`).join(" · ")
      : "";
  } catch {
    ui.billing.textContent = "";
  }
}

function setStatus(text: string, kind: "" | "running" | "error" = ""): void {
  ui.status.textContent = text;
  ui.status.className = `status ${kind}`.trim();
  ui.spinner.classList.toggle("hidden", kind !== "running");
}

// ── Commands ────────────────────────────────────────────────────────────────

let saveTimer: number | undefined;

/** Persist settings, coalesced so typing does not write on every keystroke. */
function persist(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    void invoke("save_settings", { settings }).catch(() => {
      // Losing a preference is not worth interrupting anyone over.
    });
  }, 400);
}


async function startRun(): Promise<void> {
  running = true;
  progressLog = [];
  renderPlanSummary();
  setStatus("Running — this takes tens of minutes", "running");
  ui.output.textContent = "Starting…";
  try {
    await invoke("start_run", { settings });
  } catch (error) {
    running = false;
    setStatus(String(error), "error");
    ui.output.textContent = String(error);
    renderPlanSummary();
  }
}

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

  ui.browse.addEventListener("click", async () => {
    const picked = await invoke<string | null>("pick_directory");
    if (picked) {
      settings.repo = picked;
      ui.repo.value = picked;
      refresh();
    }
  });

  ui.addModel.addEventListener("click", () => {
    settings.models = [...settings.models, { id: "", lanes: [] }];
    render();
  });

  for (const name of ["cheap", "balanced", "deep"] as Preset[]) {
    el<HTMLButtonElement>(`preset-${name}`).addEventListener("click", () => {
      settings.models = preset(name);
      render();
    });
  }

  ui.run.addEventListener("click", () => void startRun());

  // Closing the window only hides it. This is the reachable, keyboard-navigable
  // way to actually exit, so the tray is not the single point of failure.
  ui.quit.addEventListener("click", () => void invoke("quit"));
}

async function boot(): Promise<void> {
  bind();

  try {
    settings = { ...settings, ...(await invoke<Settings>("load_settings")) };
  } catch {
    // Defaults are fine; a first launch has no settings file.
  }

  applyTheme(settings.theme);
  ui.theme.value = settings.theme;
  ui.repo.value = settings.repo;
  ui.scope.value = settings.scope;
  ui.proveTop.value = String(settings.prove_top);
  ui.testCommand.value = settings.test_command;
  render();

  // Reveal as soon as the shell is painted and themed. Deliberately before the
  // event subscriptions below: if one of those ever fails, the window must
  // still appear. An app that starts invisible with no way to recover is the
  // exact failure the UX lane exists to catch.
  void invoke("frontend_ready");

  await listen<RunEvent>("run-progress", (event) => {
    progressLog.push(describe(event.payload));
    ui.output.textContent = progressLog.join(NEWLINE);
    // Keep the newest line in view without stealing focus.
    ui.output.scrollTop = ui.output.scrollHeight;
  });

  await listen<{ ok: boolean; text: string }>("run-finished", (event) => {
    running = false;
    ui.output.textContent = event.payload.text;
    setStatus(event.payload.ok ? "Finished" : "Run failed", event.payload.ok ? "" : "error");
    renderPlanSummary();
  });

  setStatus("Checking providers…", "running");
  try {
    ui.vendors.replaceChildren(...vendorPills(await invoke<VendorStatus[]>("preflight")));
    setStatus("Ready");
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
