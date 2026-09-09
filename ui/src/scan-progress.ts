import type { RunEvent } from "./format";

export interface Progress {
  total: number;
  completed: number;
  reused: number;
  failed: number;
  batch: string;
  results: string[];
}
export const initialProgress = (total: number): Progress => ({
  total,
  completed: 0,
  reused: 0,
  failed: 0,
  batch: "Waiting for provider checks",
  results: [],
});

export function advance(progress: Progress, event: RunEvent): void {
  if (event.kind === "batch_started") {
    progress.batch = `Reviewing / queued: ${event.units.join(", ")}`;
    return;
  }
  if (event.kind === "interrupted") {
    // Deliberately not counted as failures. Those reviews were never
    // attempted, so nothing was paid for them and the totals would otherwise
    // say the run tried and could not — which is what sends someone looking at
    // their own repository for a problem that is not there.
    progress.batch = `Stopped: ${event.reason} · ${event.remaining} not attempted · run again to continue`;
    return;
  }
  const label = `${event.model} · ${event.lane}`;
  if (event.kind === "reused") {
    progress.reused++;
    progress.results.push(`${label} — Reused saved review`);
  } else if (event.swept) {
    progress.completed++;
    progress.results.push(
      `${label} — Completed · ${event.findings} finding${event.findings === 1 ? "" : "s"}`,
    );
  } else {
    progress.failed++;
    progress.results.push(`${label} — Not reviewed: ${event.reason}`);
  }
  if (progress.completed + progress.reused + progress.failed === progress.total)
    progress.batch =
      "Area reviews finished · merging findings / preparing report";
}

export function progressSummary(progress: Progress): string {
  const { total, completed, reused, failed } = progress;
  return `${completed + reused + failed}/${total} reviews returned · ${completed} completed · ${reused} reused · ${failed} not reviewed`;
}

const key = (repo: string): string =>
  repo
    .replaceAll("\\", "/")
    .replace(/\/\.(?=\/|$)/g, "")
    .replace(/\/$/, "");
let states = new Map<string, Progress>();
export function beginProgress(repos: string[], total: number): void {
  states = new Map(repos.map((repo) => [key(repo), initialProgress(total)]));
  render();
}
export function updateProgress(repo: string, event: RunEvent): void {
  const state = states.get(key(repo));
  if (state) advance(state, event);
  render();
}
export function finishProgress(): void {
  for (const state of states.values())
    state.batch =
      "Scan ended — see the report for coverage and any remaining work";
  render();
}
function render(): void {
  const root = document.getElementById("scan-progress");
  if (!root) return;
  root.classList.remove("hidden");
  root.replaceChildren();
  for (const [repo, state] of states) {
    const section = document.createElement("section");
    const heading = document.createElement("h3");
    heading.textContent = repo.split("/").at(-1) ?? repo;
    heading.title = repo;
    const summary = document.createElement("p");
    summary.textContent = progressSummary(state);
    const active = document.createElement("p");
    active.className = "hint";
    active.textContent = state.batch;
    const results = document.createElement("ul");
    for (const text of state.results) {
      const item = document.createElement("li");
      item.textContent = text;
      results.append(item);
    }
    section.append(heading, summary, active, results);
    root.append(section);
  }
}
