import { beginProgress, updateProgress, finishProgress } from "./scan-progress";
import { repositories } from "./repositories";
import { unitCount } from "./model";
import { clearApplyReports } from "./apply-repositories";
/**
 * The run lifecycle: starting a sweep, reflecting its progress, showing its
 * result.
 *
 * Split from main.ts along this seam because it is the one part of the wiring
 * with real rules of its own — a late progress event must not paint over a
 * finished report, and the fix prompt must survive the window losing it.
 */

import {
  offerRepositoryResults,
  type RepositoryResult,
} from "./repository-results";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { type RunEvent, describe } from "./format";
import { findingsList } from "./findings";
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
let fixPromptPath = "";
export const currentRunReport = (): string => currentReport;
export const currentFixPrompt = (): string => fixPrompt;
export const currentFixPromptRepo = (): string => fixPromptRepo;
export const currentFixPromptPath = (): string => fixPromptPath;

export async function startRun(deps: RunDeps): Promise<void> {
  activeRunRepo = deps.settings().repo.trim();
  const resultLabel = document.getElementById("repository-result-label");
  const resultsWereShown =
    resultLabel && !resultLabel.classList.contains("hidden");
  resultLabel?.classList.add("hidden");
  running = true;
  progressLog = [];
  beginProgress(
    repositories(deps.settings()),
    unitCount(deps.settings().models),
  );
  deps.renderPlanSummary();
  deps.setStatus("Checking selected providers…", "running");
  const previousCards = [...deps.findings.children];
  const applyWasOffered = !deps.applyPanel.classList.contains("hidden");
  const reportWasOffered = !deps.copyReport.classList.contains("hidden");
  const copyWasOffered = !deps.copyPrompt.classList.contains("hidden");
  const pathWasShown = !deps.promptPath.classList.contains("hidden");
  deps.output.textContent = "Checking selected providers…";
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
    document.getElementById("scan-progress")?.classList.add("hidden");
    activeRunRepo = "";
    if (resultsWereShown) resultLabel?.classList.remove("hidden");
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

/**
 * Whether the events that end a run are being listened for.
 *
 * `run-finished` is the only thing that clears `running`. If its subscription
 * rejects — Tauri event registration failing while command IPC still works —
 * a started run never completes as far as the window is concerned, and Run,
 * Apply, Clear and Update stay blocked until the app is restarted. So the
 * button is not offered until this is true.
 */
let completionEventsReady = false;
export const runEventsReady = (): boolean => completionEventsReady;

export async function listenForRunEvents(deps: RunDeps): Promise<void> {
  await listen<RunEvent & { repo?: string }>("run-progress", (event) => {
    progressLog.push(
      (event.payload.repo ? `[${event.payload.repo}] ` : "") +
        describe(event.payload),
    );
    // Once the run has finished, the pane holds the report — the thing the
    // whole run was for. A late progress event must not paint over it.
    //
    // This is not hypothetical: the last sweep's progress event and the
    // finished event are emitted back to back, and on the first real run
    // against a large repository the progress event arrived second and
    // replaced twenty ranked defects with a log of what had just happened.
    if (!running) return;
    updateProgress(event.payload.repo ?? activeRunRepo, event.payload);
    if (progressLog.length === 1) {
      deps.setStatus("Running — this takes tens of minutes", "running");
      deps.output.textContent = "Selected providers passed pre-checks.";
    }
    // Appended, not replaced. This element is `aria-live`, and replacing its
    // whole text makes a screen reader announce the entire log again from the
    // top on every event — by the end of a run that is a hundred lines read out
    // to hear one new one. Appending announces only what is new, which is the
    // whole point of a live region.
    // The separator is decided by what is already on screen, not by how many
    // events have arrived: the pane holds the provider-check result before the
    // first one, so keying off the log's length would run that line into it.
    const line = progressLog[progressLog.length - 1] ?? "";
    const separator = deps.output.textContent === "" ? "" : NEWLINE;
    deps.output.appendChild(document.createTextNode(separator + line));
    // Keep the newest line in view without stealing focus.
    deps.output.scrollTop = deps.output.scrollHeight;
  });

  await listen<RepositoryResult>("run-finished", (event) => {
    finishProgress();
    clearApplyReports();
    running = false;
    const selectedRepo = event.payload.repo ?? activeRunRepo;
    offerRepositoryResults(event.payload, (result) => {
      showReport(result, result.repo ?? selectedRepo, deps);
      deps.setStatus(
        result.cancelled
          ? "Review stopped"
          : !result.ok
            ? "Run failed"
            : !result.complete
              ? "Review incomplete — some lanes were not swept"
              : result.saveError
                ? "Finished, but the fix prompts were not completely saved"
                : "Finished",
        result.ok && result.complete && !result.saveError ? "" : "error",
      );
    });
    // Cards first, report text underneath — the text still carries the notes
    // about unswept lanes and how severities were graded.
    // A finished run whose fix prompts did not fully save is not a plain
    // "Finished": the detail is in the output text, but the status has to say
    // the save was incomplete rather than letting the window read as all-clear.
    // Stopped is its own outcome, checked before ok. Rust returns a partial
    // report for a mid-run Stop and an error for one during pre-check, so the
    // same action used to read as "Finished" or "Run failed" by timing alone.
    // `currentReport` still keys off `ok`, because a stopped review's partial
    // report is worth copying.
    if (event.payload.cancelled) {
      deps.setStatus("Review stopped");
    } else if (!event.payload.ok) {
      deps.setStatus("Run failed", "error");
    } else if (!event.payload.complete && event.payload.saveError) {
      deps.setStatus(
        "Review incomplete — some lanes were not swept, and the fix prompts were not completely saved",
        "error",
      );
    } else if (!event.payload.complete) {
      deps.setStatus("Review incomplete — some lanes were not swept", "error");
    } else if (event.payload.saveError) {
      deps.setStatus(
        "Finished, but the fix prompts were not completely saved",
        "error",
      );
    } else {
      deps.setStatus("Finished");
    }

    // The prompt is the point of the run, so it is offered the moment there
    // is one — and its path is shown either way, because a window can be
    // closed and tens of minutes of sweeping should not go with it.
    showReport(event.payload, selectedRepo, deps);
    activeRunRepo = "";
    if (document.activeElement === deps.stop) deps.focusStatus();
    deps.renderPlanSummary();
  });

  // Both subscriptions have registered, so a started run can be heard to
  // finish. Set last, and only on the success path of both awaits.
  completionEventsReady = true;
}

function showReport(
  payload: RepositoryResult,
  repo: string,
  deps: RunDeps,
): void {
  deps.findings.replaceChildren(findingsList(payload.findings ?? []));
  deps.output.textContent = payload.text;
  currentReport = payload.ok ? payload.text : "";
  deps.copyReport.classList.toggle("hidden", currentReport === "");
  fixPrompt = payload.prompt ?? "";
  fixPromptRepo = repo;
  fixPromptPath = payload.promptPath ?? "";
  deps.copyPrompt.classList.toggle("hidden", fixPrompt === "");
  deps.applyPanel.classList.toggle("hidden", fixPromptPath === "");
  deps.promptPath.classList.toggle("hidden", fixPromptPath === "");
  deps.promptPath.textContent = fixPromptPath
    ? `Also saved to ${fixPromptPath}`
    : "";
  document.dispatchEvent(new Event("repository-report-shown"));
}

/** Saved reports are historical; reopening them never invokes a provider. */
export async function restoreReports(deps: RunDeps): Promise<void> {
  try {
    const payload = await invoke<RepositoryResult | null>(
      "load_saved_reports",
      {
        settings: deps.settings(),
      },
    );
    if (!payload || running) return;
    offerRepositoryResults(payload, (result) =>
      showReport(result, result.repo ?? "", deps),
    );
    showReport(payload, payload.repo ?? "", deps);
    deps.setStatus(
      "Saved reports restored — Run reviews missing or changed sweeps",
    );
  } catch (error) {
    deps.setStatus(`Could not reopen saved reports: ${String(error)}`, "error");
  }
}
