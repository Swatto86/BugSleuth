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

function option(value: string, label: string, selected: boolean): HTMLOptionElement {
  const opt = document.createElement("option");
  opt.value = value;
  opt.textContent = label;
  opt.selected = selected;
  return opt;
}

/**
 * The model picker for one row.
 *
 * One control, not two: an `<input list>` backed by a `<datalist>`, which is
 * the native searchable combobox. Typing narrows the suggestions in place, so
 * the search *is* the dropdown rather than a filter box sitting above it.
 *
 * This replaced a `<select>` of grouped options plus a separate filter, and it
 * deleted more than it added. A datalist cannot render group headings, so the
 * billing route moves onto each suggestion as its label — which is arguably
 * where it belonged, since it describes that one model rather than a section.
 * And because the control is a text box that happens to suggest, the "Other…"
 * escape hatch and the typed-model fallback both stop being special cases: a
 * model the catalogue has never heard of is simply typed in.
 */
function modelPicker(
  index: number,
  model: ModelSetting,
  catalogue: Catalogue,
  handlers: MatrixHandlers,
  onModelChanged: (id: string) => void,
): HTMLElement {
  const { vendor, model: current } = splitId(model.id);
  const menu = catalogue[vendor];

  const input = document.createElement("input");
  input.type = "text";
  input.value = current;
  input.spellcheck = false;
  input.autocomplete = "off";
  input.className = "model-text";
  input.placeholder = "vendor default";
  input.setAttribute("aria-label", `Model for row ${index + 1}`);
  // Every control that survives a rebuild needs one of these, not just the
  // three the first fix covered. Typing a model id while the vendor
  // catalogue finished loading in the background rebuilt the table
  // mid-keystroke and threw focus to the top of the page — on the first
  // thing anyone does after opening the app.
  input.dataset["focusKey"] = `model-${index}`;
  // Say why there are no suggestions. A box that offers nothing otherwise reads
  // as a bug rather than as a vendor that could not be asked.
  if (menu?.error) input.title = menu.error;
  input.addEventListener("input", () => {
    const id = joinId(vendor, input.value);
    handlers.onRename(index, id);
    // Which efforts apply depends on which model this is, so the effort control
    // has to follow it. Only that one cell is rebuilt: re-rendering the row on
    // every keystroke would take the focus out of this box mid-word.
    onModelChanged(id);
  });

  if (!menu || menu.groups.length === 0) return input;

  // The id must be unique per row or every box would share one row's list.
  const list = document.createElement("datalist");
  list.id = `models-${vendor}-${index}`;
  for (const group of menu.groups) {
    for (const id of group.models) {
      const opt = document.createElement("option");
      opt.value = id;
      // Shown beside the value: for Kilo this is the account the model spends
      // from, which is the difference the id alone does not make obvious.
      opt.label = group.label;
      list.append(opt);
    }
  }
  input.setAttribute("list", list.id);

  const cell = document.createElement("div");
  cell.className = "model-picker";
  cell.append(input, list);
  return cell;
}

/**
 * Which effort levels apply to one row, and why there may be none.
 *
 * Two vendors answer this about themselves and one answers about each model.
 * Claude and Codex take a CLI flag that means the same whatever model is
 * chosen. Kilo forwards `--variant` to whichever provider is behind the model,
 * so the levels are the model's: most of its models accept none at all, and
 * some accept `instant`/`thinking` rather than a ladder. Offering a
 * vendor-wide list there would put levels in the menu that the provider
 * rejects, and hide the ones it actually takes.
 *
 * The `unavailable` string is not decoration. A disabled control with no
 * explanation reads as broken, and the three reasons it can be disabled call
 * for three different responses from the user.
 */
function effortsFor(
  menu: VendorModels | undefined,
  vendor: string,
  model: string,
): { levels: string[]; unavailable: string } {
  if (!menu) {
    return { levels: [], unavailable: "still asking the CLI what it supports" };
  }
  if (menu.efforts.length > 0) {
    return { levels: menu.efforts, unavailable: "" };
  }
  if (model === "") {
    return {
      levels: [],
      unavailable: `${vendor} sets effort per model — choose a model to see its levels`,
    };
  }
  const levels = menu.efforts_by_model[model] ?? [];
  return {
    levels,
    unavailable: levels.length === 0 ? `${model} takes no effort setting` : "",
  };
}

/**
 * The effort picker for one row.
 *
 * Disabled, not hidden, when a vendor accepts no effort setting — a control
 * that silently does nothing is exactly the failure this tool exists to catch.
 */
function effortPicker(
  index: number,
  model: ModelSetting,
  catalogue: Catalogue,
  handlers: MatrixHandlers,
): HTMLSelectElement {
  const { vendor, model: current } = splitId(model.id);
  const menu = catalogue[vendor];
  const { levels, unavailable } = effortsFor(menu, vendor, current);

  const select = document.createElement("select");
  select.setAttribute("aria-label", `Effort for row ${index + 1}`);
  select.dataset["focusKey"] = `effort-${index}`;
  select.append(option("", "Default", model.effort === ""));
  for (const level of levels) {
    select.append(option(level, level, level === model.effort));
  }
  // A stored effort the catalogue does not list — because it has not loaded
  // yet, or the model's levels changed — must still show. Otherwise the row
  // reads "Default" while the run uses the stored value, and the control is
  // lying about what will happen.
  if (model.effort !== "" && !levels.includes(model.effort)) {
    select.append(option(model.effort, model.effort, true));
  }
  if (levels.length === 0) {
    select.disabled = true;
    select.title = unavailable;
  }
  select.addEventListener("change", () => handlers.onEffort(index, select.value));
  return select;
}

/**
 * How many times this model sweeps each of its lanes.
 *
 * Three identical sweeps of one fixture each returned five findings and six
 * between them: the same model finds slightly different things every time.
 * Two passes is the cheapest recall available — no second vendor needed. It
 * also costs a full sweep of quota per pass, which is why it is not the default.
 */
function passPicker(
  index: number,
  model: ModelSetting,
  handlers: MatrixHandlers,
): HTMLSelectElement {
  const select = document.createElement("select");
  select.setAttribute("aria-label", `Passes for row ${index + 1}`);
  select.dataset["focusKey"] = `passes-${index}`;
  const chosen = Math.max(1, model.passes ?? 1);
  for (const n of [1, 2, 3]) {
    select.append(option(String(n), n === 1 ? "1" : `${n}x`, n === chosen));
  }
  select.title =
    "Sweep each lane this many times. The same model finds slightly different " +
    "things each run, so repeating widens what is found — at one full sweep of " +
    "quota per pass.";
  select.addEventListener("change", () =>
    handlers.onPasses(index, Number(select.value) || 1),
  );
  return select;
}

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
      modelPicker(index, model, catalogue, handlers, (id) => {
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
        drawEffort();
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
      box.addEventListener("change", () => handlers.onToggle(index, lane, box.checked));
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
