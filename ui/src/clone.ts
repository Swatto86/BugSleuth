import { invoke } from "@tauri-apps/api/core";
import type { Settings } from "./model";

let cloning = false;
export const isCloning = (): boolean => cloning;

export function bindClone(deps: {
  settings: () => Settings;
  refresh: () => void;
  busy: () => boolean;
}): void {
  const el = <T extends HTMLElement>(id: string): T => {
    const element = document.getElementById(id);
    if (!element) throw new Error(`missing element #${id}`);
    return element as T;
  };
  const open = el<HTMLButtonElement>("clone-open");
  const dialog = el<HTMLDialogElement>("clone-dialog");
  const source = el<HTMLInputElement>("clone-source");
  const parent = el<HTMLInputElement>("clone-parent");
  const name = el<HTMLInputElement>("clone-name");
  const start = el<HTMLButtonElement>("clone-start");
  const close = el<HTMLButtonElement>("clone-close");
  const browse = el<HTMLButtonElement>("clone-browse");
  const status = el<HTMLParagraphElement>("clone-status");
  open.addEventListener("click", () => {
    if (deps.busy()) return;
    status.textContent = "";
    dialog.showModal();
    source.focus();
  });
  dialog.addEventListener("cancel", (event) => {
    if (cloning) event.preventDefault();
  });
  dialog.addEventListener("close", () => open.focus());
  close.addEventListener("click", () => {
    if (cloning) {
      void invoke("cancel_run").catch(() => {
        status.textContent =
          "Could not stop Git; wait for the clone to finish.";
      });
    } else dialog.close();
  });
  browse.addEventListener("click", () => {
    void invoke<string | null>("pick_directory").then(
      (picked) => {
        if (picked) parent.value = picked;
      },
      (error: unknown) => {
        status.textContent = String(error);
      },
    );
  });
  start.addEventListener("click", () => {
    if (deps.busy() || cloning) return;
    if (![source, parent, name].every((input) => input.reportValidity()))
      return;
    cloning = true;
    close.textContent = "Stop cloning";
    close.focus();
    for (const control of [source, parent, name, start, browse])
      control.disabled = true;
    status.textContent =
      "Cloning repository… Git may open its authentication helper.";
    deps.refresh();
    let selected = false;
    void invoke<string>("clone_repository", {
      source: source.value,
      parent: parent.value,
      name: name.value,
    })
      .then((repo) => {
        deps.settings().repo = repo;
        deps.settings().scope = "";
        el<HTMLInputElement>("repo").value = repo;
        el<HTMLInputElement>("scope").value = "";
        source.value = "";
        selected = true;
      })
      .catch((error: unknown) => {
        status.textContent = String(error);
      })
      .finally(() => {
        cloning = false;
        for (const control of [source, parent, name, start, browse])
          control.disabled = false;
        close.textContent = "Close";
        deps.refresh();
        if (selected) dialog.close();
      });
  });
}
