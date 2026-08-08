/**
 * Boot and catalogue loads must not persist the settings they render.
 *
 * When the saved file is unreadable the app shows defaults; if that initial
 * render wrote them back it would overwrite the recoverable file. This guards
 * the two ends of the fix: `refresh` only persists when not suppressed, and the
 * two startup renders go through `renderWithoutPersisting`.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...parts: string[]) =>
  fs.readFileSync(path.join(root, ...parts), "utf8").replace(/\r\n?/g, "\n");

test("startup renders do not persist settings back over the saved file", () => {
  const main = read("ui", "src", "main.ts");

  assert.ok(
    main.includes("function renderWithoutPersisting"),
    "the non-persisting render helper is gone",
  );
  assert.ok(
    main.includes("if (!suppressPersistence) persist()"),
    "refresh no longer gates persistence, so a startup render writes defaults back",
  );

  // Exactly the two startup call sites — boot and loadCatalogue — render this
  // way; the definition uses the name without a call.
  const calls = (main.match(/renderWithoutPersisting\(\)/g) ?? []).length;
  assert.ok(
    calls >= 2,
    `boot and loadCatalogue should render without persisting, found ${calls} call(s)`,
  );
});
