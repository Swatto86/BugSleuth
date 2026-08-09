/**
 * What editing a row of the model matrix does to the settings.
 *
 * Split from main.ts at the hard line cap, along the seam already there:
 * model.ts holds the rules, view.ts draws the table, and this is the third
 * part — what each control in a row changes, and what has to be confirmed
 * before it is lost.
 */

import { supportsAgents, type Settings, toggleLane } from "./model";
import { confirmDialog } from "./dialog";
import type { MatrixHandlers } from "./view";

export interface MatrixDeps {
  settings: () => Settings;
  /** Rebuild the table: the row set or a row's menus changed. */
  render: () => void;
  /** Redraw everything that depends on state but not on the table's identity. */
  refresh: () => void;
}

export function matrixHandlers({
  settings,
  render,
  refresh,
}: MatrixDeps): MatrixHandlers {
  return {
    onRename: (index, id) => {
      const models = settings().models;
      const existing = models[index];
      if (existing) models[index] = { ...existing, id };
      refresh();
    },
    onVendor: (index, id) => {
      const models = settings().models;
      const existing = models[index];
      // The effort goes with the old vendor: the levels differ between them,
      // and carrying one over would send a value the new CLI may reject.
      if (existing)
        models[index] = {
          ...existing,
          id,
          effort: "",
          use_agents: supportsAgents(id) && (existing.use_agents ?? false),
        };
      render();
    },
    onAgents: (index, useAgents) => {
      const models = settings().models;
      const existing = models[index];
      if (existing) models[index] = { ...existing, use_agents: useAgents };
      refresh();
    },
    onPasses: (index, passes) => {
      const models = settings().models;
      const existing = models[index];
      if (existing) models[index] = { ...existing, passes };
      refresh();
    },
    onEffort: (index, effort) => {
      const models = settings().models;
      const existing = models[index];
      if (existing) models[index] = { ...existing, effort };
      refresh();
    },
    onToggle: (index, lane, on) => {
      const state = settings();
      state.models = toggleLane(state.models, index, lane, on);
      render();
    },
    onRemove: (index) => {
      const model = settings().models[index];
      if (!model) return;
      void confirmDialog({
        title: "Remove this model?",
        message:
          `This removes ${model.id || `row ${index + 1}`} and all of its ` +
          "lane, agent, effort, and pass settings. This cannot be undone.",
        confirmLabel: "Remove model",
        destructive: true,
      }).then((yes) => {
        if (!yes) return;
        // Read again rather than closing over the settings object: the dialog
        // is awaited, and the stored settings can be replaced while it is up.
        const state = settings();
        state.models = state.models.filter((_, i) => i !== index);
        render();
      });
    },
  };
}
