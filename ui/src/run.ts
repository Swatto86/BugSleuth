/**
 * The run lifecycle: starting a sweep, reflecting its progress, showing its
 * result.
 *
 * Split from main.ts along this seam because it is the one part of the wiring
 * with real rules of its own — a late progress event must not paint over a
 * finished report, and the fix prompt must survive the window losing it.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { type RunEvent, describe } from "./format";
import { type FindingCard, findingsList } from "./findings";
import type { Settings } from "./model";

const NEWLINE = String.fromCharCode(10);

/** What the lifecycle needs from the window, handed in rather than imported. */
export interface RunDeps {
  output: HTMLPreElement;
  stop: HTMLButtonElement;
  findings: HTMLDivElement;
  copyPrompt: HTMLButtonElement;
  promptPath: HTMLParagraphElement;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  renderPlanSummary: () => void;
  settings: () => Settings;
}

/** Lines accumulated during a run, newest last. */
let progressLog: string[] = [];

let running = false;
export const isRunning = (): boolean => running;

/** The last run's fix prompt, held so the Copy button has something to give. */
let fixPrompt = "";
export const currentFixPrompt = (): string => fixPrompt;

export async function startRun(deps: RunDeps): Promise<void> {
  running = true;
  progressLog = [];
  deps.renderPlanSummary();
  deps.setStatus("Running — this takes tens of minutes", "running");
  deps.output.textContent = "Starting…";
  deps.findings.replaceChildren();
  // Re-enabled per run: it disables itself once pressed, so a second press
  // cannot arrive while the first is still killing processes.
  deps.stop.disabled = false;
  try {
    await invoke("start_run", { settings: deps.settings() });
  } catch (error) {
    running = false;
    deps.setStatus(String(error), "error");
    deps.output.textContent = String(error);
    deps.renderPlanSummary();
  }
}

export async function listenForRunEvents(deps: RunDeps): Promise<void> {
  await listen<RunEvent>("run-progress", (event) => {
    progressLog.push(describe(event.payload));
    // Once the run has finished, the pane holds the report — the thing the
    // whole run was for. A late progress event must not paint over it.
    //
    // This is not hypothetical: the last sweep's progress event and the
    // finished event are emitted back to back, and on the first real run
    // against a large repository the progress event arrived second and
    // replaced twenty ranked defects with a log of what had just happened.
    if (!running) return;
    deps.output.textContent = progressLog.join(NEWLINE);
    // Keep the newest line in view without stealing focus.
    deps.output.scrollTop = deps.output.scrollHeight;
  });

  await listen<{
    ok: boolean;
    text: string;
    prompt?: string;
    promptPath?: string | null;
    findings?: FindingCard[];
  }>("run-finished", (event) => {
    running = false;
    // Cards first, report text underneath — the text still carries the notes
    // about unswept lanes and how severities were graded.
    deps.findings.replaceChildren(findingsList(event.payload.findings ?? []));
    deps.output.textContent = event.payload.text;
    deps.setStatus(event.payload.ok ? "Finished" : "Run failed", event.payload.ok ? "" : "error");

    // The prompt is the point of the run, so it is offered the moment there
    // is one — and its path is shown either way, because a window can be
    // closed and tens of minutes of sweeping should not go with it.
    fixPrompt = event.payload.prompt ?? "";
    deps.copyPrompt.classList.toggle("hidden", fixPrompt === "");
    const path = event.payload.promptPath;
    deps.promptPath.classList.toggle("hidden", !path);
    deps.promptPath.textContent = path ? `Also saved to ${path}` : "";
    deps.renderPlanSummary();
  });
}
