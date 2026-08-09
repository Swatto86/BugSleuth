/**
 * The confirmation dialog must associate its warning paragraph so screen readers
 * announce the destructive-action detail, not just the title and the focused
 * button. The actual announcement is exercised by a screen reader; this reads
 * dialog.ts's own syntax so a regression to an unassociated body is caught
 * without a live app.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import ts from "typescript";

import { frontendFiles, walk } from "./ast.test.ts";

function functionText(source: ts.SourceFile, name: string): string | undefined {
  let text: string | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) {
      text = node.getText();
    }
  });
  return text;
}

test("the confirmation dialog describes its message to assistive tech", () => {
  const dialog = frontendFiles().find((file) => file.fileName === "dialog.ts");
  assert.ok(dialog, "dialog.ts is no longer a shipped frontend module");

  const confirmDialog = functionText(dialog, "confirmDialog");
  assert.ok(confirmDialog, "confirmDialog() is gone from dialog.ts");

  assert.match(
    confirmDialog,
    /body\.id\s*=/,
    "the dialog body paragraph has no id for aria-describedby to reference",
  );
  assert.match(
    confirmDialog,
    /aria-describedby/,
    "the dialog panel is not described by its warning paragraph",
  );
});

test("Tab with focus outside the dialog's buttons re-enters the trap", () => {
  const dialog = frontendFiles().find((file) => file.fileName === "dialog.ts");
  assert.ok(dialog, "dialog.ts is no longer a shipped frontend module");
  const confirmDialog = functionText(dialog, "confirmDialog");
  assert.ok(confirmDialog, "confirmDialog() is gone from dialog.ts");
  assert.match(
    confirmDialog,
    /active !== first && active !== last/,
    "Tab from <body> escapes the modal into the app behind the overlay",
  );
});
