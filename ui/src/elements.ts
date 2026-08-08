/**
 * Every element the window touches, looked up once.
 *
 * Split out of `main.ts` at the hard line cap, along the seam that was already
 * there: this is the whole of what the app knows about the markup, and the rest
 * of that file is about what to do with it.
 *
 * Looked up eagerly and loudly. A missing id throws at startup rather than
 * producing an `undefined` that fails later at a click, which is how a renamed
 * element used to survive as far as a user pressing a button that did nothing.
 * `styles.test.ts` checks every id named here exists in `index.html`.
 */

const el = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing element #${id}`);
  return found as T;
};

const ui = {
  theme: el<HTMLSelectElement>("theme"),
  vendors: el<HTMLDivElement>("vendors"),
  checkSignin: el<HTMLButtonElement>("check-signin"),
  checkUpdate: el<HTMLButtonElement>("check-update"),
  repo: el<HTMLInputElement>("repo"),
  scope: el<HTMLInputElement>("scope"),
  browse: el<HTMLButtonElement>("browse"),
  matrixBody: el<HTMLTableSectionElement>("matrix-body"),
  addModel: el<HTMLButtonElement>("add-model"),
  uncovered: el<HTMLDivElement>("uncovered-warning"),
  proveEnabled: el<HTMLInputElement>("prove-enabled"),
  proveSettings: el<HTMLDivElement>("prove-settings"),
  proveTop: el<HTMLInputElement>("prove-top"),
  testCommand: el<HTMLInputElement>("test-command"),
  reuseCompleted: el<HTMLInputElement>("reuse-completed"),
  clearSaved: el<HTMLButtonElement>("clear-saved"),
  triageSeverities: el<HTMLInputElement>("triage-severities"),
  output: el<HTMLPreElement>("output"),
  findings: el<HTMLDivElement>("findings"),
  status: el<HTMLSpanElement>("status"),
  settingsError: el<HTMLSpanElement>("settings-error"),
  spinner: el<HTMLSpanElement>("spinner"),
  planSummary: el<HTMLSpanElement>("plan-summary"),
  run: el<HTMLButtonElement>("run"),
  stop: el<HTMLButtonElement>("stop"),
  quit: el<HTMLButtonElement>("quit"),
  copyPrompt: el<HTMLButtonElement>("copy-prompt"),
  promptPath: el<HTMLParagraphElement>("prompt-path"),
  applyPanel: el<HTMLDivElement>("apply-panel"),
  applyVendor: el<HTMLSelectElement>("apply-vendor"),
  applyModel: el<HTMLDivElement>("apply-model"),
  applyEffort: el<HTMLDivElement>("apply-effort"),
  applyFixes: el<HTMLButtonElement>("apply-fixes"),
  pushAfterApply: el<HTMLInputElement>("push-after-apply"),
  tagReleaseAfterPush: el<HTMLInputElement>("tag-release-after-push"),
};

export { el, ui };
