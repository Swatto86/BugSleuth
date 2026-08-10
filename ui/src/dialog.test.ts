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

function declaration(
  source: ts.SourceFile,
  name: string,
): ts.FunctionDeclaration | undefined {
  let found: ts.FunctionDeclaration | undefined;
  walk(source, (node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) {
      found = node;
    }
  });
  return found;
}

test("the confirmation dialog describes its message to assistive tech", () => {
  const dialog = frontendFiles().find((file) => file.fileName === "dialog.ts");
  assert.ok(dialog, "dialog.ts is no longer a shipped frontend module");

  const confirmDialog = declaration(dialog, "confirmDialog");
  assert.ok(confirmDialog, "confirmDialog() is gone from dialog.ts");

  assert.match(
    confirmDialog.getText(),
    /body\.id\s*=/,
    "the dialog body paragraph has no id for aria-describedby to reference",
  );
  // The IDREF's target, not just the attribute's name. Matching bare
  // "aria-describedby" passed a panel described by its own heading, which
  // announces the title twice and never reads the destructive-action warning.
  let describedByBody = false;
  walk(confirmDialog, (node) => {
    if (!ts.isCallExpression(node)) return;
    const [attribute, target] = node.arguments;
    describedByBody =
      describedByBody ||
      (ts.isPropertyAccessExpression(node.expression) &&
        ts.isIdentifier(node.expression.expression) &&
        node.expression.expression.text === "panel" &&
        node.expression.name.text === "setAttribute" &&
        attribute !== undefined &&
        ts.isStringLiteral(attribute) &&
        attribute.text === "aria-describedby" &&
        target !== undefined &&
        ts.isPropertyAccessExpression(target) &&
        ts.isIdentifier(target.expression) &&
        target.expression.text === "body" &&
        target.name.text === "id");
  });
  assert.ok(
    describedByBody,
    "the dialog panel is not described by its warning paragraph",
  );
});

test("Tab with focus outside the dialog's buttons re-enters the trap", () => {
  const dialog = frontendFiles().find((file) => file.fileName === "dialog.ts");
  assert.ok(dialog, "dialog.ts is no longer a shipped frontend module");
  const confirmDialog = declaration(dialog, "confirmDialog");
  assert.ok(confirmDialog, "confirmDialog() is gone from dialog.ts");
  assert.match(
    confirmDialog.getText(),
    /active !== first && active !== last/,
    "Tab from <body> escapes the modal into the app behind the overlay",
  );
});
