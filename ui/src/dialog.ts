/**
 * An in-app confirmation, because `window.confirm` is not one.
 *
 * The browser dialog looks like a browser inside a desktop app, cannot be
 * themed, and — the part that actually matters — blocks the whole webview,
 * which means a run's progress events queue up behind it. It was also the wrong
 * answer for a plainer reason: this app has a dark theme and a tray, and a grey
 * Chrome prompt in the middle of it says "web page", not "application".
 *
 * Kept deliberately small. It is a modal with a title, a message, two buttons
 * and the keyboard behaviour a desktop dialog is expected to have:
 *
 * - **Escape cancels**, because a modal you cannot dismiss from the keyboard is
 *   the trap the UX lane exists to find.
 * - **Focus starts on the safe button**, so a stray Enter does not confirm a
 *   destructive action.
 * - **Tab is trapped inside**, or focus walks out to the still-visible app
 *   behind the overlay and the user acts on controls the modal is meant to be
 *   guarding.
 * - **Focus returns** to whatever opened it, so keyboard position is not lost.
 */
export interface ConfirmRequest {
  title: string;
  message: string;
  /** Label for the action that goes ahead. Names the act, never "OK". */
  confirmLabel: string;
  /** True when confirming destroys something, so it can be styled as such. */
  destructive?: boolean;
}

export function confirmDialog(request: ConfirmRequest): Promise<boolean> {
  return new Promise((resolve) => {
    const opener = document.activeElement as HTMLElement | null;

    const overlay = document.createElement("div");
    overlay.className = "dialog-overlay";

    const panel = document.createElement("div");
    panel.className = "dialog";
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-modal", "true");

    const heading = document.createElement("h2");
    heading.id = `dialog-title-${Date.now()}`;
    heading.textContent = request.title;
    panel.setAttribute("aria-labelledby", heading.id);

    const body = document.createElement("p");
    body.id = `dialog-message-${heading.id}`;
    body.textContent = request.message;
    panel.setAttribute("aria-describedby", body.id);

    const buttons = document.createElement("div");
    buttons.className = "dialog-buttons";

    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.textContent = "Cancel";

    const confirm = document.createElement("button");
    confirm.type = "button";
    confirm.textContent = request.confirmLabel;
    if (request.destructive) confirm.className = "danger";

    buttons.append(cancel, confirm);
    panel.append(heading, body, buttons);
    overlay.append(panel);

    const close = (answer: boolean) => {
      document.removeEventListener("keydown", onKey, true);
      overlay.remove();
      // Put the caller back where they were, or a keyboard user restarts from
      // the top of the document every time they decline something.
      opener?.focus?.();
      resolve(answer);
    };

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close(false);
        return;
      }
      if (event.key !== "Tab") return;
      // Trap: without this, Tab leaves the dialog and reaches the application
      // behind it, which is still fully interactive.
      const stops = [cancel, confirm];
      const first = stops[0]!;
      const last = stops[stops.length - 1]!;
      const active = document.activeElement;
      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      } else if (active !== first && active !== last) {
        // A click on the dialog's text lands focus on <body>; Tab must
        // re-enter the trap, not walk the app behind the overlay.
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      }
    };

    cancel.addEventListener("click", () => close(false));
    confirm.addEventListener("click", () => close(true));
    // A click on the backdrop is a dismissal, not a confirmation.
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) close(false);
    });
    document.addEventListener("keydown", onKey, true);

    document.body.append(overlay);
    // The safe choice takes focus, so Enter never confirms by accident.
    cancel.focus();
  });
}
