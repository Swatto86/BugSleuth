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
  copyReport: HTMLButtonElement;
  copyPrompt: HTMLButtonElement;
  promptPath: HTMLParagraphElement;
  /** Where the fixes are handed to a model. Offered once there is a prompt. */
  applyPanel: HTMLDivElement;
  setStatus: (text: string, kind?: "" | "running" | "error") => void;
  focusStatus: () => void;
  renderPlanSummary: () => void;
  settings: () => Settings;
}

/** Lines accumulated during a run, newest last. */
let progressLog: string[] = [];

let running = false;
export const isRunning = (): boolean => running;

/** The last run's fix prompt, held so the Copy button has something to give. */
let activeRunRepo = "";
let currentReport = "";
let fixPrompt = "";
let fixPromptRepo = "";
export const currentRunReport = (): string => currentReport;
export const currentFixPrompt = (): string => fixPrompt;
export const currentFixPromptRepo = (): string => fixPromptRepo;

export async function startRun(deps: RunDeps): Promise<void> {
  activeRunRepo = deps.settings().repo.trim();
  running = true;
  progressLog = [];
  deps.renderPlanSummary();
  deps.setStatus("Running — this takes tens of minutes", "running");
  const previousCards = [...deps.findings.children];
  const applyWasOffered = !deps.applyPanel.classList.contains("hidden");
  const reportWasOffered = !deps.copyReport.classList.contains("hidden");
  const copyWasOffered = !deps.copyPrompt.classList.contains("hidden");
  const pathWasShown = !deps.promptPath.classList.contains("hidden");
  deps.output.textContent = "Starting…";
  deps.findings.replaceChildren();
  // The panel applies *the last run's* prompt, and that file is about to be
  // rewritten. Offering it during a run would apply a report the pane is no
  // longer showing.
  deps.applyPanel.classList.add("hidden");
  deps.copyReport.classList.add("hidden");
  deps.copyPrompt.classList.add("hidden");
  deps.promptPath.classList.add("hidden");
  // Re-enabled per run: it disables itself once pressed, so a second press
  // cannot arrive while the first is still killing processes.
  deps.stop.disabled = false;
  // renderPlanSummary above disables the Run button, and in WebView2 disabling
  // the focused element drops focus to <body> — stranding a keyboard or
  // screen-reader user at the top of the page, unable to reach the live Stop
  // control without tabbing through the whole form. Move focus to Stop, which
  // has just become visible and enabled.
  deps.stop.focus();
  try {
    await invoke("start_run", { settings: deps.settings() });
  } catch (error) {
    activeRunRepo = "";
    running = false;
    deps.setStatus(String(error), "error");
    deps.output.textContent = String(error);
    deps.findings.replaceChildren(...previousCards);
    if (applyWasOffered) deps.applyPanel.classList.remove("hidden");
    if (reportWasOffered) deps.copyReport.classList.remove("hidden");
    if (copyWasOffered) deps.copyPrompt.classList.remove("hidden");
    if (pathWasShown) deps.promptPath.classList.remove("hidden");
    if (document.activeElement === deps.stop) deps.focusStatus();
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
    // Appended, not replaced. This element is `aria-live`, and replacing its
    // whole text makes a screen reader announce the entire log again from the
    // top on every event — by the end of a run that is a hundred lines read out
    // to hear one new one. Appending announces only what is new, which is the
    // whole point of a live region.
    // The separator is decided by what is already on screen, not by how many
    // events have arrived: the pane is seeded with "Starting…" before the first
    // one, so keying off the log's length would run the first line into it.
    const line = progressLog[progressLog.length - 1] ?? "";
    const separator = deps.output.textContent === "" ? "" : NEWLINE;
    deps.output.appendChild(document.createTextNode(separator + line));
    // Keep the newest line in view without stealing focus.
    deps.output.scrollTop = deps.output.scrollHeight;
  });

  await listen<{
    ok: boolean;
    text: string;
    prompt?: string;
    promptPath?: string | null;
    saveError?: string;
    findings?: FindingCard[];
  }>("run-finished", (event) => {
    running = false;
    // Cards first, report text underneath — the text still carries the notes
    // about unswept lanes and how severities were graded.
    deps.findings.replaceChildren(findingsList(event.payload.findings ?? []));
    deps.output.textContent = event.payload.text;
    currentReport = event.payload.text;
    deps.copyReport.classList.toggle("hidden", currentReport === "");
    // A finished run whose fix prompts did not fully save is not a plain
    // "Finished": the detail is in the output text, but the status has to say
    // the save was incomplete rather than letting the window read as all-clear.
    if (event.payload.ok && event.payload.saveError) {
      deps.setStatus(
        "Finished, but the fix prompts were not completely saved",
        "error",
      );
    } else {
      deps.setStatus(
        event.payload.ok ? "Finished" : "Run failed",
        event.payload.ok ? "" : "error",
      );
    }

    // The prompt is the point of the run, so it is offered the moment there
    // is one — and its path is shown either way, because a window can be
    // closed and tens of minutes of sweeping should not go with it.
    fixPrompt = event.payload.prompt ?? "";
    fixPromptRepo = activeRunRepo;
    activeRunRepo = "";
    deps.copyPrompt.classList.toggle("hidden", fixPrompt === "");
    // Applying reads the prompt Rust wrote to disk, so it is offered only when
    // this run actually produced one.
    deps.applyPanel.classList.toggle("hidden", !event.payload.promptPath);
    const path = event.payload.promptPath;
    deps.promptPath.classList.toggle("hidden", !path);
    deps.promptPath.textContent = path ? `Also saved to ${path}` : "";
    if (document.activeElement === deps.stop) deps.focusStatus();
    deps.renderPlanSummary();
  });
}
