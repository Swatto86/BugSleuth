import type { Settings } from "./model";

export const repositoryLines = (text: string): string[] => [
  ...new Set(
    text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean),
  ),
];

export const repositories = (settings: Settings): string[] =>
  repositoryLines(
    [settings.repo, ...(settings.additional_repos ?? [])].join("\n"),
  );

/** Keep the existing settings format readable by earlier releases. */
export function setRepositories(settings: Settings, paths: string[]): void {
  const [first = "", ...rest] = repositoryLines(paths.join("\n"));
  settings.repo = first;
  settings.additional_repos = rest;
}

export function clonePlan(
  text: string,
  folder: string,
  existing: number,
): { source: string; name: string }[] {
  const sources = repositoryLines(text);
  if (!sources.length)
    throw new Error("Enter at least one repository address.");
  if (sources.length + existing > 16)
    throw new Error("Choose at most 16 repositories in total.");
  if (sources.length > 1 && folder.trim())
    throw new Error(
      "Leave the folder override empty when cloning multiple repositories.",
    );
  const plan = sources.map((source) => ({
    source,
    name:
      folder.trim() ||
      source
        .replace(/[\\/]+$/, "")
        .split(/[\\/:]/)
        .at(-1)!
        .replace(/\.git$/, ""),
  }));
  if (
    plan.some(
      ({ name }) =>
        !name ||
        name.startsWith(".") ||
        /[\\/:<>"|?*\x00-\x1f]|[. ]$/.test(name),
    )
  )
    throw new Error(
      "Could not infer a safe folder name. Clone that repository separately with a folder override.",
    );
  if (new Set(plan.map(({ name }) => name.toLowerCase())).size !== plan.length)
    throw new Error(
      "Repositories have the same folder name. Clone them separately with different folder overrides.",
    );
  return plan;
}
