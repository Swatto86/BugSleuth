/**
 * The "how many to attempt" field must show the value the run will use.
 *
 * The input handler clamps prove_top to [1,25] but never wrote the clamp back,
 * so the box could show 50 while the run attempted 25, with nothing telling the
 * user. A change listener reconciles the field on commit; this guards it exists.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { frontendFiles } from "./ast.test.ts";

test("the prove-top field is reconciled to the value the run will use", () => {
  const controls = frontendFiles().find((f) => f.fileName === "controls.ts");
  assert.ok(controls, "controls.ts is no longer shipped");
  const text = controls.getText();
  assert.match(
    text,
    /proveTop\.addEventListener\("change"/,
    "no change listener reconciles the prove-top field",
  );
  assert.match(
    text,
    /ui\.proveTop\.value =/,
    "the prove-top field value is never written back, so it can show a number the run will not use",
  );
});
