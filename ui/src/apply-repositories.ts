import type { ApplyDeps } from "./apply";
import { splitId, type Settings } from "./model";

export type ApplyChoice = Pick<Settings, "apply_model" | "apply_effort">;
export const activeApplies = new Set<string>();
const logs = new Map<string, string[]>();
const summaries = new Map<string, string>();
export const applyLog = (repo: string): string =>
  (logs.get(repo) ?? []).join("\n\n");

/** Persist each report's choices without changing the scan target or defaults. */
export function repositoryDeps(original: ApplyDeps): ApplyDeps {
  const choices = new Map<string, Settings>();
  const settings = (): Settings => {
    const root = original.settings();
    const repo = original.promptRepo();
    if (!repo) return root;
    let stored = choices.get(repo);
    if (!stored) {
      stored = { ...root, ...root.apply_repositories?.[repo] };
      choices.set(repo, stored);
    }
    return stored;
  };
  return {
    ...original,
    settings,
    refresh: () => {
      const repo = original.promptRepo();
      if (repo) {
        const { apply_model, apply_effort } = settings();
        (original.settings().apply_repositories ??= {})[repo] = {
          apply_model,
          apply_effort,
        };
      }
      original.refresh();
    },
  };
}

export function recordApply(
  repo: string,
  model: string,
  text: string,
  status: string,
): void {
  const entries = logs.get(repo) ?? [];
  entries.push(text);
  logs.set(repo, entries);
  const selected = splitId(model);
  summaries.set(
    repo,
    `${repo} — ${selected.vendor}:${selected.model}: ${status}`,
  );
  let summary = document.getElementById("apply-jobs");
  if (!summary) {
    summary = document.createElement("pre");
    summary.id = "apply-jobs";
    summary.setAttribute("role", "status");
    document.getElementById("output")?.before(summary);
  }
  summary.textContent = [...summaries.values()].join("\n");
}

export function clearApplyReports(): void {
  logs.clear();
  summaries.clear();
  document.getElementById("apply-jobs")?.remove();
}
