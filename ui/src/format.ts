/**
 * Turning engine output into the words a person reads.
 *
 * No DOM and no Tauri here, so the wording rules can be tested directly.
 * That matters more than it looks: the difference between 'swept and found
 * nothing' and 'never ran' is the single most important thing this product
 * has to say clearly, and it is decided in these few lines.
 */

/** Mirrors `RunEvent` in the engine. */
export type RunEvent =
  | { kind: "batch_started"; index: number; total: number; units: string[] }
  | { kind: "reused"; model: string; lane: string }
  | {
      kind: "sweep_finished";
      model: string;
      lane: string;
      findings: number;
      swept: boolean;
      reason: string;
    };

/**
 * Turn one engine event into a line.
 *
 * A sweep that did not run says so and why. It is never rendered as a count of
 * zero findings, which would read as "looked and found nothing".
 */
export function describe(event: RunEvent): string {
  switch (event.kind) {
    case "batch_started":
      return `Round ${event.index}/${event.total}: ${event.units.join(", ")}`;
    case "reused":
      return `  reused ${event.model} × ${event.lane} from an earlier run`;
    case "sweep_finished":
      return event.swept
        ? `  ${event.model} × ${event.lane}: ${event.findings} finding${event.findings === 1 ? "" : "s"}`
        : `  ${event.model} × ${event.lane}: NOT SWEPT — ${event.reason}`;
  }
}

/** Join names the way English does: a, b and c. */
export function listOf(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}
