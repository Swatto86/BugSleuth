/**
 * Rendering the ranked defects as cards.
 *
 * The report text stays available underneath — the notes about unswept lanes
 * and how severities were graded belong to it — but a wall of text cannot be
 * acted on per defect. A card can: it shows the grade, where the defect is,
 * who found it, and expands to the full account with the fix approach.
 */

export interface FindingCard {
  position: number;
  severity: string;
  /** The finder's own grade, present only when a triage pass moved it. */
  claimedSeverity?: string | null;
  triageReason?: string | null;
  title: string;
  file: string;
  line: number;
  agreement: number;
  models: string[];
  explanation: string;
  failure: string;
  fixApproach: string;
  snippet: string;
  /** A complete, standalone fix prompt for this one defect. */
  prompt?: string;
}

/** Build the whole findings list. Empty input renders nothing at all. */
export function findingsList(cards: FindingCard[]): HTMLElement {
  const list = document.createElement("div");
  list.className = "findings";
  for (const card of cards) list.append(findingCard(card));
  return list;
}

function findingCard(card: FindingCard): HTMLElement {
  // <details> gives expand/collapse, keyboard toggling, and screen-reader
  // state announcements without a line of behaviour code.
  const details = document.createElement("details");
  details.className = "finding";

  const summary = document.createElement("summary");
  summary.append(
    badge(card.severity),
    text("span", "finding-title", `${card.position}. ${card.title}`),
  );
  details.append(summary);

  const body = document.createElement("div");
  body.className = "finding-body";

  const where = text("p", "finding-where", `${card.file}:${card.line}`);
  body.append(where);

  body.append(
    text(
      "p",
      "finding-meta",
      card.agreement > 1
        ? `Found independently by ${card.agreement} models: ${card.models.join(", ")}`
        : `Found by ${card.models.join(", ")}`,
    ),
  );

  // A moved grade is shown with what it moved from and why. Hiding it would
  // present the triage pass's opinion as if the finder had said it.
  if (card.claimedSeverity) {
    body.append(
      text(
        "p",
        "finding-regrade",
        `Re-graded: the model that found it said ${card.claimedSeverity}` +
          (card.triageReason ? ` — ${card.triageReason}` : ""),
      ),
    );
  }

  body.append(section("Why it is wrong", card.explanation));
  body.append(section("How it fails", card.failure));
  if (card.snippet.trim()) {
    const pre = document.createElement("pre");
    pre.className = "finding-snippet";
    pre.textContent = card.snippet;
    body.append(sectionTitled("The code", pre));
  }
  if (card.fixApproach.trim()) {
    body.append(section("Fix approach", card.fixApproach));
  }
  if (card.prompt) body.append(copyButton(card));

  details.append(body);
  return details;
}

/**
 * Copy this one defect's work order.
 *
 * The whole bundle is offered above, but a small local model handed twenty
 * defects at once truncates and answers confidently about the first few. One
 * defect is the unit the handoff was designed around, so it is the unit the
 * window should hand over.
 */
function copyButton(card: FindingCard): HTMLElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy-one";
  const idle = "Copy this defect's fix prompt";
  button.textContent = idle;
  button.addEventListener("click", () => {
    void navigator.clipboard.writeText(card.prompt ?? "").then(
      () => {
        // Confirmed rather than assumed: a copy that silently failed and one
        // that worked look identical, and the user finds out by pasting
        // nothing into an agent.
        button.textContent = "Copied";
        window.setTimeout(() => (button.textContent = idle), 1500);
      },
      (error: unknown) => {
        button.textContent = `Could not copy: ${String(error)}`;
      },
    );
  });
  return button;
}

function badge(severity: string): HTMLElement {
  const span = text("span", `sev sev-${severity}`, severity.toUpperCase());
  return span;
}

function section(title: string, content: string): HTMLElement {
  return sectionTitled(title, text("p", "", content));
}

function sectionTitled(title: string, content: HTMLElement): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "finding-section";
  wrap.append(text("h4", "", title), content);
  return wrap;
}

function text(tag: string, className: string, value: string): HTMLElement {
  const el = document.createElement(tag);
  if (className) el.className = className;
  el.textContent = value;
  return el;
}
