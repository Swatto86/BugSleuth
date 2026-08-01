/**
 * Wiring: DOM in, commands out.
 *
 * The rules live in model.ts; this file only reflects state into elements and
 * sends what the user asked for to Rust.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  LANES,
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

/** Mirrors `RunEvent` in the engine. */
type RunEvent =
  | { kind: "batch_started"; index: number; total: number; units: string[] }
  | { kind: "reused"; model: string; lane: string }
  | {
      kind: "sweep_finished";
      model: string;
      lane: string;
      findings: number;
      swept: boolean;
      reason: string;
    };

interface VendorStatus {
  name: string;
  available: boolean;
  detail: string;
}

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

function renderVendors(statuses: VendorStatus[]): void {
  ui.vendors.replaceChildren(
    ...statuses.map((vendor) => {
      const pill = document.createElement("span");
      pill.className = `pill ${vendor.available ? "ok" : "bad"}`;
      const dot = document.createElement("span");
      dot.className = "dot";
      const name = document.createElement("span");
      name.textContent = vendor.name;
      const detail = document.createElement("span");
      detail.className = "detail";
      detail.textContent = vendor.available ? vendor.detail : "unavailable";
      pill.append(dot, name, detail);
      pill.title = vendor.detail;
      return pill;
    }),
  );
}

function renderMatrix(): void {
  ui.matrixBody.replaceChildren(
    ...settings.models.map((model, index) => {
      const row = document.createElement("tr");

      const idCell = document.createElement("td");
      idCell.className = "model-id";
      const idInput = document.createElement("input");
      idInput.type = "text";
      idInput.value = model.id;
      idInput.spellcheck = false;
      idInput.setAttribute("aria-label", `Model ${index + 1} identifier`);
      idInput.addEventListener("input", () => {
        settings.models[index] = { ...model, id: idInput.value };
        refresh();
      });
      idCell.append(idInput);
      row.append(idCell);

      for (const lane of LANES) {
        const cell = document.createElement("td");
        cell.className = "lane-cell";
        const box = document.createElement("input");
        box.type = "checkbox";
        box.checked = model.lanes.includes(lane);
        box.setAttribute(
          "aria-label",
          `${model.id || "model"} covers the ${LANE_TITLES[lane]} lane`,
        );
        box.addEventListener("change", () => {
          settings.models = toggleLane(settings.models, index, lane as Lane, box.checked);
          render();
        });
        cell.append(box);
        row.append(cell);
      }

      const removeCell = document.createElement("td");
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "Remove";
      remove.setAttribute("aria-label", `Remove ${model.id || "this model"}`);
      remove.addEventListener("click", () => {
        settings.models = settings.models.filter((_, i) => i !== index);
        render();
      });
      removeCell.append(remove);
      row.append(removeCell);

      return row;
    }),
  );
}

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

function listOf(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

function renderPlanSummary(): void {
  const units = unitCount(settings.models);
  const rounds = batchCount(settings.models);
  ui.planSummary.textContent =
    units === 0 ? "" : `${units} sweep${units === 1 ? "" : "s"} · ${rounds} round${rounds === 1 ? "" : "s"}`;
  ui.run.disabled = running || !canRun(settings);
}

function render(): void {
  renderMatrix();
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

/** Persist settings, coalesced so typing does not write on every keystroke. */
function persist(): void {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    void invoke("save_settings", { settings }).catch(() => {
      // Losing a preference is not worth interrupting anyone over.
    });
  }, 400);
}

/**
 * Turn one engine event into a line.
 *
 * A sweep that did not run says so and why. It is never rendered as a count of
 * zero findings, which would read as "looked and found nothing".
 */
function describe(event: RunEvent): string {
  switch (event.kind) {
    case "batch_started":
      return `Round ${event.index}/${event.total}: ${event.units.join(", ")}`;
    case "reused":
      return `  reused ${event.model} × ${event.lane} from an earlier run`;
    case "sweep_finished":
      return event.swept
        ? `  ${event.model} × ${event.lane}: ${event.findings} finding${event.findings === 1 ? "" : "s"}`
        : `  ${event.model} × ${event.lane}: NOT SWEPT — ${event.reason}`;
  }
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
    renderVendors(await invoke<VendorStatus[]>("preflight"));
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
