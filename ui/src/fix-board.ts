import type { ApplyDeps } from "./apply";
import {
  activeApplies,
  applySummary,
  repositorySettings,
} from "./apply-repositories";
import { repositoryReports } from "./repository-results";
import { offeredVendors, vendorCliPresent } from "./cli-offer";
import { effortIsValid, joinId, splitId, type Vendor } from "./model";
import { effortPicker, modelPicker, option } from "./pickers";

/** Repository assignments stay visible while any individual report is open. */
export function bindFixBoard(deps: ApplyDeps, redraw: () => void) {
  const board = document.getElementById("fix-board")!;
  const rows = document.getElementById("fix-repositories")!;
  let ready = false;
  const refresh = (eventsReady: boolean): void => {
    ready = eventsReady;
    board.classList.toggle("hidden", !repositoryReports.length || deps.busy());
    for (const row of rows.querySelectorAll<HTMLElement>("[data-repo]")) {
      const repo = row.dataset["repo"]!;
      const stored = repositorySettings(deps.settings(), repo);
      const active = activeApplies.has(repo);
      const button = row.querySelector<HTMLButtonElement>("[data-fix-start]")!;
      button.disabled =
        !ready ||
        deps.busy() ||
        active ||
        !stored.apply_model.trim() ||
        row.dataset["saved"] !== "true" ||
        !vendorCliPresent(stored.apply_model, deps.catalogue()) ||
        !effortIsValid(
          stored.apply_model,
          stored.apply_effort,
          deps.catalogue(),
        );
      button.textContent = active ? "Fix in progress…" : "Apply fixes…";
      row.querySelector(".fix-state")!.textContent =
        row.dataset["saved"] === "true"
          ? applySummary(repo)
          : "No saved fix prompt — run a review first";
      row.querySelector<HTMLFieldSetElement>("fieldset")!.disabled = active;
    }
  };
  const draw = (): void => {
    const input =
      document.activeElement instanceof HTMLInputElement
        ? document.activeElement
        : undefined;
    const selection = input
      ? ([input.selectionStart, input.selectionEnd] as const)
      : undefined;
    const focused =
      document.activeElement instanceof HTMLElement &&
      rows.contains(document.activeElement)
        ? document.activeElement.dataset["focusKey"]
        : undefined;
    rows.replaceChildren();
    repositoryReports.forEach((result, index) => {
      const repo = result.repo;
      if (!repo) return;
      const stored = repositorySettings(deps.settings(), repo);
      const row = document.createElement("article");
      row.className = "fix-repository";
      row.dataset["repo"] = repo;
      row.dataset["saved"] = String(Boolean(result.promptPath));
      const title = document.createElement("h3");
      title.textContent = repo;
      const controls = document.createElement("fieldset");
      controls.className = "row";
      controls.setAttribute("aria-label", `Fix assignment for ${repo}`);
      const save = (): void => {
        const { apply_model, apply_effort } = stored;
        (deps.settings().apply_repositories ??= {})[repo] = {
          apply_model,
          apply_effort,
        };
        deps.refresh();
      };
      const field = (label: string, control: HTMLElement): void => {
        const container = document.createElement("label");
        container.className = "field";
        const caption = document.createElement("span");
        caption.textContent = label;
        container.append(caption, control);
        controls.append(container);
      };
      const vendor = document.createElement("select");
      vendor.dataset["fixVendor"] = "";
      vendor.dataset["focusKey"] = `fix-vendor-${index}`;
      const selected = splitId(stored.apply_model).vendor;
      vendor.append(
        ...offeredVendors(deps.catalogue(), selected).map((name) =>
          option(name, name, name === selected),
        ),
      );
      vendor.onchange = () => {
        stored.apply_model = joinId(vendor.value as Vendor, "");
        stored.apply_effort = "";
        save();
        redraw();
      };
      field("Provider", vendor);
      field(
        "Model",
        modelPicker({
          key: `fix-model-${index}`,
          label: `Fixing model for ${repo}`,
          id: stored.apply_model,
          catalogue: deps.catalogue(),
          onChange: (id) => {
            stored.apply_model = id;
            if (!effortIsValid(id, stored.apply_effort, deps.catalogue()))
              stored.apply_effort = "";
            save();
            redraw();
          },
        }),
      );
      field(
        "Effort",
        effortPicker({
          key: `fix-effort-${index}`,
          label: `Fixing effort for ${repo}`,
          id: stored.apply_model,
          effort: stored.apply_effort,
          catalogue: deps.catalogue(),
          onChange: (effort) => {
            stored.apply_effort = effort;
            save();
            redraw();
          },
        }),
      );
      const show = (): void => {
        const select = document.getElementById(
          "repository-result",
        ) as HTMLSelectElement;
        if (select.options.length) {
          select.value = String(index + 1);
          select.dispatchEvent(new Event("change"));
        }
      };
      const report = document.createElement("button");
      report.type = "button";
      report.textContent = "View report";
      report.onclick = show;
      const apply = document.createElement("button");
      apply.type = "button";
      apply.className = "primary";
      apply.dataset["fixStart"] = "";
      apply.onclick = () => {
        show();
        deps.ui.button.click();
      };
      const status = document.createElement("p");
      status.className = "hint fix-state";
      status.setAttribute("role", "status");
      row.append(title, controls, report, apply, status);
      rows.append(row);
    });
    refresh(ready);
    if (focused)
      rows
        .querySelector<HTMLElement>(`[data-focus-key="${CSS.escape(focused)}"]`)
        ?.focus();
    if (
      focused &&
      selection &&
      document.activeElement instanceof HTMLInputElement
    )
      document.activeElement.setSelectionRange(selection[0], selection[1]);
  };
  return { draw, refresh };
}
