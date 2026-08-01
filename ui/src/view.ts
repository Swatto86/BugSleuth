/**
 * Turning data into elements.
 *
 * Everything this file needs is passed in, so it knows nothing about the app's
 * state or which command anything invokes. main.ts owns those; this owns only
 * what the user sees.
 */

import { LANES, LANE_TITLES, type Lane, type ModelSetting } from "./model";

export interface VendorStatus {
  name: string;
  available: boolean;
  detail: string;
}

/** One pill per provider CLI. */
export function vendorPills(statuses: VendorStatus[]): HTMLElement[] {
  return statuses.map((vendor) => {
    const pill = document.createElement("span");
    pill.className = `pill ${vendor.available ? "ok" : "bad"}`;
    const dot = document.createElement("span");
    dot.className = "dot";
    const name = document.createElement("span");
    name.textContent = vendor.name;
    const detail = document.createElement("span");
    detail.className = "detail";
    // A CLI that cannot start says so plainly rather than showing a version
    // string nobody can act on.
    detail.textContent = vendor.available ? vendor.detail : "unavailable";
    pill.append(dot, name, detail);
    pill.title = vendor.detail;
    return pill;
  });
}

export interface MatrixHandlers {
  onRename: (index: number, id: string) => void;
  onToggle: (index: number, lane: Lane, on: boolean) => void;
  onRemove: (index: number) => void;
}

/** One row per configured model, with a checkbox per lane. */
export function matrixRows(
  models: ModelSetting[],
  handlers: MatrixHandlers,
): HTMLTableRowElement[] {
  return models.map((model, index) => {
    const row = document.createElement("tr");

    const idCell = document.createElement("td");
    idCell.className = "model-id";
    const idInput = document.createElement("input");
    idInput.type = "text";
    idInput.value = model.id;
    idInput.spellcheck = false;
    idInput.setAttribute("aria-label", `Model ${index + 1} identifier`);
    idInput.addEventListener("input", () => handlers.onRename(index, idInput.value));
    idCell.append(idInput);
    row.append(idCell);

    for (const lane of LANES) {
      const cell = document.createElement("td");
      cell.className = "lane-cell";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = model.lanes.includes(lane);
      // Named for a screen reader, which cannot see the column heading.
      box.setAttribute(
        "aria-label",
        `${model.id || "model"} covers the ${LANE_TITLES[lane]} lane`,
      );
      box.addEventListener("change", () => handlers.onToggle(index, lane, box.checked));
      cell.append(box);
      row.append(cell);
    }

    const removeCell = document.createElement("td");
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remove";
    remove.setAttribute("aria-label", `Remove ${model.id || "this model"}`);
    remove.addEventListener("click", () => handlers.onRemove(index));
    removeCell.append(remove);
    row.append(removeCell);

    return row;
  });
}
