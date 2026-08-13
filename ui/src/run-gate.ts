/**
 * Whether a configuration can start a review, and the reason when it cannot.
 *
 * Split from model.ts at the hard line cap. Re-exported from there so callers
 * still import `canRun` / `runBlockReason` from "./model".
 */

import { vendorCliPresent } from "./cli-offer.ts";
import {
  CANNOT_DELEGATE,
  LANES,
  MAX_PASSES,
  effortIsValid,
  supportsAgents,
  unitCount,
  usesUltracode,
  vendorOf,
  type Settings,
} from "./model.ts";
import type { Catalogue } from "./view.ts";

/**
 * Why this configuration cannot run, or `null` when it can.
 *
 * Predicates and order match `canRun` as it used to be written: a boolean
 * disable left keyboard and screen-reader users with a dead Run button and
 * no explanation.
 */
export function runBlockReason(
  settings: Settings,
  catalogue: Catalogue,
): string | null {
  if (settings.repo.trim().length === 0) {
    return "Choose a repository folder first.";
  }
  if (settings.models.length === 0) {
    return "Add at least one model with a lane ticked.";
  }
  for (const model of settings.models) {
    if (model.id.trim() === "") {
      return "Every row needs a model id — finish or remove the empty row.";
    }
    if (!vendorCliPresent(model.id, catalogue)) {
      return `The ${vendorOf(model.id)} CLI is not installed on this machine.`;
    }
    if (!effortIsValid(model.id, model.effort, catalogue)) {
      return "A model has an effort it does not accept; set that row's effort to Default.";
    }
    if (model.use_agents && !supportsAgents(model.id)) {
      const vendor = vendorOf(model.id);
      return `Turn Agents off for ${vendor}; ${CANNOT_DELEGATE.join(", ")} cannot delegate a review.`;
    }
    if (
      model.use_agents &&
      usesUltracode(model.id) &&
      model.effort.trim() !== ""
    ) {
      return "Claude Ultracode replaces explicit effort; set that row's effort to Default.";
    }
    if (
      !model.lanes.every((lane) => (LANES as readonly string[]).includes(lane))
    ) {
      return "A model has a lane the engine does not know; remove the unknown lane.";
    }
    if ((model.passes ?? 1) > MAX_PASSES) {
      return `A model asks for more than ${MAX_PASSES} passes; the engine will refuse.`;
    }
  }
  if (unitCount(settings.models) === 0) {
    return "Tick at least one lane so there is something to sweep.";
  }
  return null;
}

/** Whether a configuration could run at all. */
export function canRun(settings: Settings, catalogue: Catalogue): boolean {
  return runBlockReason(settings, catalogue) === null;
}
