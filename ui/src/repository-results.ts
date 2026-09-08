import type { FindingCard } from "./findings";

export interface RepositoryResult {
  repo?: string;
  ok: boolean;
  complete: boolean;
  cancelled: boolean;
  text: string;
  prompt?: string;
  promptPath?: string | null;
  saveError?: string;
  findings?: FindingCard[];
  results?: RepositoryResult[];
}

/** Keep the overview and every report available without changing the run inputs. */
export function offerRepositoryResults(
  payload: RepositoryResult,
  show: (result: RepositoryResult) => void,
): void {
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
