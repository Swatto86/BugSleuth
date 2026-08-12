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
  joinId,
  splitId,
  supportsAgents,
  usesUltracode,
  type Lane,
  type ModelGroup,
  type ModelSetting,
  type Vendor,
} from "./model";
import { offeredVendors } from "./cli-offer.ts";

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
  /** Whether this vendor's CLI is installed on the machine. */
  installed: boolean;
  error: string | null;
}

export type Catalogue = Record<string, VendorModels | undefined>;

export interface MatrixHandlers {
  onRename: (index: number, id: string) => void;
  /** Changing vendor swaps which menu applies, so the table is rebuilt. */
  onVendor: (index: number, id: string) => void;
  onEffort: (index: number, effort: string) => void;
  onPasses: (index: number, passes: number) => void;
  onAgents: (index: number, useAgents: boolean) => void;
  onToggle: (index: number, lane: Lane, on: boolean) => void;
  onRemove: (index: number) => void;
}

import { effortPicker, modelPicker, option, passPicker } from "./pickers";

/** One row per model: provider, model, agents, effort, passes, then lanes. */
export function matrixRows(
  models: ModelSetting[],
  catalogue: Catalogue,
  handlers: MatrixHandlers,
): HTMLTableRowElement[] {
  return models.map((model, index) => {
    const row = document.createElement("tr");
    const { vendor } = splitId(model.id);
    const agentsSupported = supportsAgents(model.id);

    const vendorCell = document.createElement("td");
    const vendorSelect = document.createElement("select");
    vendorSelect.setAttribute("aria-label", `Provider for row ${index + 1}`);
    // Rebuilding the table replaces every element, so keyboard focus lands
    // on <body>. These keys let the caller put it back where it was.
    vendorSelect.dataset["focusKey"] = `vendor-${index}`;
    for (const name of offeredVendors(catalogue, vendor)) {
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
    const laneControls: Array<{ box: HTMLInputElement; lane: Lane }> = [];
    let agentControl: HTMLInputElement | undefined;
    let removeControl: HTMLButtonElement | undefined;
    const updateRowActionLabels = (id: string): void => {
      const selected = splitId(id);
      const name = selected.model || selected.vendor;
      for (const { box, lane } of laneControls) {
        box.setAttribute(
          "aria-label",
          `${name}, row ${index + 1}, covers the ${LANE_TITLES[lane]} lane`,
        );
      }
      agentControl?.setAttribute(
        "aria-label",
        agentsSupported
          ? `Use parallel review agents for ${name}, row ${index + 1}`
          : `Parallel review agents unavailable for ${name}, row ${index + 1}: ${selected.vendor} cannot delegate`,
      );
      if (agentControl) {
        const title = !agentsSupported
          ? `Unavailable: ${selected.vendor} has no subagent mode BugSleuth can ask for.`
          : selected.vendor === "claude" && usesUltracode(id)
            ? "Use Claude Ultracode with two parallel foreground agents for this lane (provider limit: 16 concurrent). Uses more tokens."
            : `Ask ${selected.vendor === "claude" ? "Claude Code" : "Codex"} to delegate independent parts of this lane in parallel. Uses more tokens.`;
        agentControl.title = title;
        agentControl.parentElement?.setAttribute("title", title);
      }
      removeControl?.setAttribute(
        "aria-label",
        `Remove ${name} from row ${index + 1}`,
      );
    };
    const drawEffort = () => {
      const picker = effortPicker({
        key: `effort-${index}`,
        label: `Effort for row ${index + 1}`,
        id: live.id,
        effort: live.effort,
        catalogue,
        forced:
          live.use_agents && usesUltracode(live.id)
            ? {
                label: "Ultracode",
                title:
                  "Claude agent mode uses Ultracode (xhigh reasoning plus two foreground subagents).",
              }
            : undefined,
        onChange: (effort) => {
          live = { ...live, effort };
          handlers.onEffort(index, effort);
        },
      });
      effortCell.replaceChildren(picker);
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
          updateRowActionLabels(id);
          // Which efforts apply depends on which model this is, so that control
          // has to follow it. Only that one cell is rebuilt: re-rendering the row
          // on every keystroke would take focus out of the box mid-word.
          drawEffort();
        },
      }),
    );
    const passCell = document.createElement("td");
    passCell.className = "effort-cell";
    passCell.append(passPicker(index, model, handlers));

    const agentCell = document.createElement("td");
    agentCell.className = "agent-cell";
    const agentBox = document.createElement("input");
    agentBox.type = "checkbox";
    agentBox.dataset["focusKey"] = `agents-${index}`;
    agentBox.checked = agentsSupported && (model.use_agents ?? false);
    agentBox.disabled = !agentsSupported;
    agentBox.title = agentsSupported
      ? vendor === "claude" && usesUltracode(model.id)
        ? "Use Claude Ultracode with two parallel foreground agents for this lane (provider limit: 16 concurrent). Uses more tokens."
        : `Ask ${vendor === "claude" ? "Claude Code" : "Codex"} to delegate independent parts of this lane in parallel. Uses more tokens.`
      : "Unavailable: BugSleuth uses Kilo's read-only Ask agent, which cannot delegate.";
    agentCell.title = agentBox.title;
    agentBox.addEventListener("change", () =>
      handlers.onAgents(index, agentBox.checked),
    );
    agentControl = agentBox;
    agentCell.append(agentBox);
    row.append(idCell, agentCell, effortCell, passCell);

    for (const lane of LANES) {
      const cell = document.createElement("td");
      cell.className = "lane-cell";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.dataset["focusKey"] = `lane-${index}-${lane}`;
      box.checked = model.lanes.includes(lane);
      laneControls.push({ box, lane });
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
    removeControl = remove;
    remove.dataset["focusKey"] = `remove-${index}`;
    remove.addEventListener("click", () => handlers.onRemove(index));
    updateRowActionLabels(live.id);
    removeCell.append(remove);
    row.append(removeCell);

    return row;
  });
}
