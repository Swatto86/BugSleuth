/**
 * Turning data into elements.
 *
 * Everything this file needs is passed in, so it knows nothing about the app's
 * state or which command anything invokes. main.ts owns those; this owns only
 * what the user sees.
 */

import {
  LANES,
  LANE_TITLES,
  VENDORS,
  joinId,
  splitId,
  type Lane,
  type ModelGroup,
  type ModelSetting,
  type Vendor,
} from "./model";

export interface VendorStatus {
  name: string;
  available: boolean;
  detail: string;
}

/**
 * One pill per vendor, after a real sign-in check.
 *
 * Distinct from `vendorPills`, which reports whether a CLI can be *started* —
 * a weaker fact that every one of these tools satisfies while logged out. The
 * detail carries the vendor's own words, because "run `claude login`" is what
 * actually tells someone what to do and a generic "unavailable" throws it away.
 */
export function signinPills(
  results: { name: string; available: boolean; detail: string }[],
): HTMLElement[] {
  return results.map((result) => {
    const pill = document.createElement("span");
    pill.className = `pill ${result.available ? "ok" : "bad"}`;
    const dot = document.createElement("span");
    dot.className = "dot";
    const label = document.createElement("span");
    label.textContent = result.detail;
    pill.append(dot, label);
    // The full sentence is already the label; the title repeats it for a
    // narrow window where the pill has to clip.
    pill.title = result.detail;
    return pill;
  });
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

/** One vendor's menu, as the backend describes it. */
export interface VendorModels {
  vendor: string;
  groups: ModelGroup[];
  /** Efforts the CLI accepts, for vendors where that is a CLI-wide property. */
  efforts: string[];
  /** Efforts a particular model accepts, for vendors where it is per model. */
  efforts_by_model: Record<string, string[] | undefined>;
  error: string | null;
}

export type Catalogue = Record<string, VendorModels | undefined>;

export interface MatrixHandlers {
  onRename: (index: number, id: string) => void;
  /** Changing vendor swaps which menu applies, so the table is rebuilt. */
  onVendor: (index: number, id: string) => void;
  onEffort: (index: number, effort: string) => void;
  onPasses: (index: number, passes: number) => void;
  onToggle: (index: number, lane: Lane, on: boolean) => void;
  onRemove: (index: number) => void;
}

import { effortPicker, modelPicker, option, passPicker } from "./pickers";

/** One row per configured model: provider, model, effort, passes, then a lane per column. */
export function matrixRows(
  models: ModelSetting[],
  catalogue: Catalogue,
  handlers: MatrixHandlers,
): HTMLTableRowElement[] {
  return models.map((model, index) => {
    const row = document.createElement("tr");
    const { vendor, model: current } = splitId(model.id);

    const vendorCell = document.createElement("td");
    const vendorSelect = document.createElement("select");
    vendorSelect.setAttribute("aria-label", `Provider for row ${index + 1}`);
    // Rebuilding the table replaces every element, so keyboard focus lands
    // on <body>. These keys let the caller put it back where it was.
    vendorSelect.dataset["focusKey"] = `vendor-${index}`;
    for (const name of VENDORS) {
      vendorSelect.append(option(name, name, name === vendor));
    }
    vendorSelect.addEventListener("change", () => {
      // Changing vendor drops the model: an id from one vendor is meaningless
      // to another, and carrying it over would send a real-looking model id to
      // a CLI that has never heard of it.
      handlers.onVendor(index, joinId(vendorSelect.value as Vendor, ""));
    });
    vendorCell.append(vendorSelect);
    row.append(vendorCell);

    const effortCell = document.createElement("td");
    effortCell.className = "effort-cell";

    // The row's own view of what is selected, so the effort cell can be rebuilt
    // without waiting for the whole table to re-render.
    let live: ModelSetting = model;
    const drawEffort = () => {
      effortCell.replaceChildren(
        effortPicker(index, live, catalogue, {
          ...handlers,
          onEffort: (i, effort) => {
            live = { ...live, effort };
            handlers.onEffort(i, effort);
          },
        }),
      );
    };
    drawEffort();

    const idCell = document.createElement("td");
    idCell.className = "model-id";
    idCell.append(
      modelPicker({
        key: `model-${index}`,
        label: `Model for row ${index + 1}`,
        id: model.id,
        catalogue,
        onChange: (id) => {
          handlers.onRename(index, id);
          // A model that does not accept the effort already chosen must not keep
          // it: it would be sent to the CLI and rejected. Clearing is the honest
          // reset — different model, different levels.
          const { vendor: v, model: m } = splitId(id);
          const allowed = catalogue[v]?.efforts.length
            ? catalogue[v].efforts
            : (catalogue[v]?.efforts_by_model[m] ?? []);
          const effort = allowed.includes(live.effort) ? live.effort : "";
          if (effort !== live.effort) handlers.onEffort(index, effort);
          live = { ...live, id, effort };
          // Which efforts apply depends on which model this is, so that control
          // has to follow it. Only that one cell is rebuilt: re-rendering the row
          // on every keystroke would take focus out of the box mid-word.
          drawEffort();
        },
      }),
    );
    row.append(idCell);
    row.append(effortCell);

    const passCell = document.createElement("td");
    passCell.className = "effort-cell";
    passCell.append(passPicker(index, model, handlers));
    row.append(passCell);

    for (const lane of LANES) {
      const cell = document.createElement("td");
      cell.className = "lane-cell";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.dataset["focusKey"] = `lane-${index}-${lane}`;
      box.checked = model.lanes.includes(lane);
      // Named for a screen reader, which cannot see the column heading.
      box.setAttribute(
        "aria-label",
        `${current || vendor} covers the ${LANE_TITLES[lane]} lane`,
      );
      box.addEventListener("change", () =>
        handlers.onToggle(index, lane, box.checked),
      );
      cell.append(box);
      row.append(cell);
    }

    const removeCell = document.createElement("td");
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remove";
    remove.setAttribute("aria-label", `Remove ${current || vendor}`);
    remove.dataset["focusKey"] = `remove-${index}`;
    remove.addEventListener("click", () => handlers.onRemove(index));
    removeCell.append(remove);
    row.append(removeCell);

    return row;
  });
}
