import type { FindingCard } from "./findings";

export interface RepositoryResult {
  repo?: string;
  ok: boolean;
  complete: boolean;
  cancelled: boolean;
  /**
   * Why the run gave up, when nobody stopped it.
   *
   * Distinct from `cancelled` and from `complete`. A spent usage allowance
   * leaves lanes that were never attempted rather than lanes that failed, so
   * nothing has been paid for them and running again picks up exactly those.
   * Absent on a run that reached its end.
   */
  interrupted?: string | null;
  text: string;
  prompt?: string;
  promptPath?: string | null;
  saveError?: string;
  findings?: FindingCard[];
  results?: RepositoryResult[];
}

export let repositoryReports: RepositoryResult[] = [];

/** Forget every report: the selector empties and the fix board has nothing to draw. */
export function forgetRepositoryResults(): void {
  repositoryReports = [];
  const label = document.getElementById("repository-result-label");
  const select = document.getElementById(
    "repository-result",
  ) as HTMLSelectElement | null;
  label?.classList.add("hidden");
  if (select) {
    select.replaceChildren();
    select.onchange = null;
  }
}

/** Keep the overview and every report available without changing the run inputs. */
export function offerRepositoryResults(
  payload: RepositoryResult,
  show: (result: RepositoryResult) => void,
): void {
  repositoryReports = payload.results ?? [payload];
  const label = document.getElementById("repository-result-label");
  const select = document.getElementById(
    "repository-result",
  ) as HTMLSelectElement | null;
  if (!label || !select) return;
  label.classList.toggle("hidden", !payload.results?.length);
  select.replaceChildren();
  select.onchange = null;
  if (!payload.results?.length) return;
  const results = [payload, ...payload.results];
  for (const [index, result] of results.entries()) {
    const option = document.createElement("option");
    option.value = String(index);
    const status = result.cancelled
      ? "Stopped"
      : !result.ok
        ? "Failed"
        : !result.complete
          ? "Incomplete"
          : result.saveError
            ? "Save failed"
            : "Finished";
    option.textContent =
      index === 0
        ? "Batch overview"
        : `${result.repo ?? "Repository"} — ${status}`;
    select.append(option);
  }
  select.onchange = () => {
    const result = results[Number(select.value)];
    if (result) show(result);
  };
}
