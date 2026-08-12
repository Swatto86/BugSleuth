/**
 * Which provider CLIs the menus may offer.
 *
 * Kept out of `model.ts` so that file stays under the line cap, and out of
 * `view.ts` so the install gate can be tested without building DOM.
 */

import { VENDORS, splitId, type Vendor } from "./model.ts";

/** The install flags the backend sends per vendor — enough to filter menus. */
export type InstallCatalogue = Record<
  string,
  { installed: boolean } | undefined
>;

/**
 * Providers the menus may offer, given which CLIs are installed.
 *
 * Until the catalogue has loaded there is nothing to filter on, so every known
 * vendor stays visible rather than blanking the table during boot. After load,
 * only installed CLIs appear — plus the row's current vendor when it is no
 * longer installed, so a stale saved setting can still be seen and changed.
 */
export function offeredVendors(
  catalogue: InstallCatalogue,
  current?: Vendor,
): Vendor[] {
  if (Object.keys(catalogue).length === 0) return [...VENDORS];
  const installed = VENDORS.filter((name) => catalogue[name]?.installed);
  if (current && !installed.includes(current)) {
    return [...installed, current];
  }
  return [...installed];
}

/**
 * Providers the sweep-matrix menus may offer.
 *
 * Codex can still apply fixes, but it cannot run a repository review. New
 * sweep rows therefore omit it; a saved Codex row stays visible so it can be
 * switched away from.
 */
export function offeredSweepVendors(
  catalogue: InstallCatalogue,
  current?: Vendor,
): Vendor[] {
  return offeredVendors(catalogue, current).filter(
    (name) => name !== "codex" || current === "codex",
  );
}

/**
 * Whether this model's vendor CLI is present on the machine.
 *
 * Permissive while the catalogue has not loaded, and permissive when a vendor
 * is simply absent from a partial catalogue — only an explicit `installed:
 * false` blocks. That is what the backend sends for a missing CLI.
 */
export function vendorCliPresent(
  id: string,
  catalogue: InstallCatalogue,
): boolean {
  if (Object.keys(catalogue).length === 0) return true;
  const { vendor } = splitId(id);
  const menu = catalogue[vendor];
  if (!menu) return true;
  return menu.installed;
}
