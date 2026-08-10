/**
 * Drawing the plan: which lanes are uncovered, and keeping focus while the
 * table underneath it is rebuilt.
 *
 * Split from `main.ts` at the hard line cap. Both of these are about what the
 * user sees and where their keyboard is, and neither needs the command wiring
 * that the rest of that file is: they take what they need as arguments.
 */

import { ui } from "./elements";
import { listOf } from "./format";
import {
  LANE_TITLES,
  type Lane,
  type ModelSetting,
  uncoveredLanes,
} from "./model";

/**
 * Surface uncovered lanes prominently, and mark the column heads.
 *
 * This is the UI's most important job. The report will say "not swept" either
 * way, but by then the run is paid for; here it is still free to fix.
 */
export function renderCoverage(models: ModelSetting[]): void {
  const uncovered = uncoveredLanes(models);

  for (const head of document.querySelectorAll<HTMLElement>("th.lane-cell")) {
    const lane = head.dataset["lane"] ?? "";
    head.classList.toggle("uncovered", uncovered.includes(lane as Lane));
  }

  if (uncovered.length === 0) {
    ui.uncovered.classList.add("hidden");
    ui.uncovered.textContent = "";
    return;
  }
  const names = uncovered.map((lane) => LANE_TITLES[lane]);
  ui.uncovered.classList.remove("hidden");
  ui.uncovered.textContent =
    `No model covers ${listOf(names)}. ${uncovered.length === 1 ? "That lane" : "Those lanes"} ` +
    `will be reported as NOT SWEPT — nothing will be looked for there, which is not ` +
    `the same as nothing being wrong.`;
}

/**
 * Rebuild the model table, putting keyboard focus back where it was.
 *
 * Toggling a lane replaces every element in the table, so focus fell to
 * `<body>` — on the app's busiest control, a keyboard user was thrown back to
 * the top of the page on every single tick. There is no workaround for that
 * except tabbing all the way in again, each time.
 *
 * The identity is a `data-focus-key` the row builder writes. Restoring by
 * position would put focus on a different lane the moment a row is removed.
 */
export function withRestoredFocus(rowCount: number, rebuild: () => void): void {
  const focused = document.activeElement;
  const key =
    focused instanceof HTMLElement ? focused.dataset["focusKey"] : undefined;

  rebuild();

  if (key === undefined) return;
  const restored = ui.matrixBody.querySelector<HTMLElement>(
    `[data-focus-key="${CSS.escape(key)}"]`,
  );
  if (restored) {
    restored.focus();
    return;
  }

  // Removing the focused final row deletes its focus key. Continue at the new
  // final row, or at Add model when the matrix is now empty — otherwise focus
  // falls to <body> and a keyboard user restarts from the top of the page.
  if (key.startsWith("remove-")) {
    const lastRemove = ui.matrixBody.querySelector<HTMLElement>(
      `[data-focus-key="remove-${rowCount - 1}"]`,
    );
    (lastRemove ?? ui.addModel).focus();
  }
}
