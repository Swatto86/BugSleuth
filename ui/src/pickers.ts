/**
 * The controls for one row of the model matrix.
 *
 * Split out of `view.ts` when that file hit its size cap. The seam is real
 * rather than convenient: these four build the cells of a single row and know
 * about vendors, catalogues and effort levels, while what remains in `view.ts`
 * turns lists into elements and knows none of that.
 *
 * Types come back from `view.ts` as type-only imports, which are erased, so
 * there is no import cycle at runtime.
 */

import { joinId, splitId, type ModelSetting } from "./model";
import type { Catalogue, MatrixHandlers, VendorModels } from "./view";

export function option(
  value: string,
  label: string,
  selected: boolean,
): HTMLOptionElement {
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
export function modelPicker(
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

  const everything: { value: string; label: string }[] = [];
  for (const group of menu?.groups ?? []) {
    for (const id of group.models) {
      everything.push({ value: id, label: group.label });
    }
  }
  if (everything.length === 0) return input;

  // The id must be unique per row or every box would share one row's list.
  const list = document.createElement("datalist");
  list.id = `models-${vendor}-${index}`;

  /**
   * Show the suggestions that contain what has been typed, anywhere in them.
   *
   * Filtered here rather than left to the browser. A `<datalist>` narrows its
   * own options, but by rules that are engine-defined and unspecified — so
   * whether typing `glm` finds `zai-coding/glm-5.2` was a property of whichever
   * WebView happened to be installed rather than of this app. It did not.
   *
   * Model ids carry the interesting part in the middle: a provider prefix, then
   * the family, then the version. Matching only the start means knowing how an
   * id begins before you can search for it, which defeats searching.
   */
  const fill = (query: string): void => {
    const needle = query.trim().toLowerCase();
    const matches = everything.filter(
      (m) =>
        needle === "" || `${m.value} ${m.label}`.toLowerCase().includes(needle),
    );
    list.replaceChildren(
      ...matches.slice(0, 60).map((m) => {
        const opt = document.createElement("option");
        opt.value = m.value;
        // Shown beside the value: for Kilo this is the account the model spends
        // from, which is the difference the id alone does not make obvious.
        opt.label = m.label;
        return opt;
      }),
    );
  };
  fill(current);
  input.addEventListener("input", () => {
    fill(input.value);
  });
  input.setAttribute("list", list.id);

  const cell = document.createElement("div");
  cell.className = "model-picker";
  cell.append(input, list);
  return cell;
}

/**
 * Which effort levels apply to one row, and why there may be none.
 *
 * One vendor answers this about itself and two answer about each model.
 * Claude takes a CLI flag that means the same whatever model is chosen. Kilo
 * forwards `--variant` to whichever provider is behind the model, so the levels
 * are the model's: most accept none at all, and some accept `instant`/`thinking`
 * rather than a ladder. Codex is per-model too — `codex debug models` reports
 * `ultra` for `gpt-5.6-sol` and stops at `xhigh` for `gpt-5.5`.
 *
 * Offering a vendor-wide list where the answer is per model puts levels in the
 * menu that the provider rejects, and hides the ones it actually takes. This
 * comment said Codex was CLI-wide until the catalogue was read and disagreed.
 *
 * The `unavailable` string is not decoration. A disabled control with no
 * explanation reads as broken, and the three reasons it can be disabled call
 * for three different responses from the user.
 */
export function effortsFor(
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
export function effortPicker(
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
  select.addEventListener("change", () =>
    handlers.onEffort(index, select.value),
  );
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
export function passPicker(
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
