/**
 * The wording of the one confirmation that matters.
 *
 * Split from `apply.ts` at the hard line cap, and it earns a file of its own:
 * this is the last thing between a click and a model with write access to
 * someone's checkout — and, when publishing is on, between a click and commits
 * on a remote that cannot be recalled. What it says has to change with what
 * will actually happen, which is why it is built rather than written once.
 */

import type { ConfirmRequest } from "./dialog";
import type { Settings } from "./model";

export function applyConfirmation(
  repo: string,
  settings: Settings,
): ConfirmRequest {
  const publishing = settings.push_after_apply;
  const releasing = publishing && settings.tag_release_after_push;
  return {
    title: releasing
      ? "Apply these fixes, push them, and tag a release?"
      : publishing
        ? "Apply these fixes and push them?"
        : "Apply these fixes to your code?",
    message:
      `This runs the displayed fix prompt against "${repo}" using ${settings.apply_model} with write access, ` +
      "editing files in place and running your tests. It is refused unless " +
      "the working tree is clean, so everything it does will show up in " +
      "`git diff` and `git log` — but nothing it writes has been checked by " +
      "anyone. Read the changes before you keep them. Each defect is a " +
      "separate run, and every one that finishes is recorded — so if this " +
      "stops part-way, pressing Apply again continues rather than starting " +
      "over." +
      (publishing
        ? " Whatever it commits will then be pushed to this branch's " +
          "upstream. That part cannot be undone: once the commits are on " +
          "the remote, anyone watching it can fetch them, and a later " +
          "rewrite does not recall them."
        : "") +
      (releasing
        ? " The pushed commits will then be tagged with the next patch " +
          "version, which starts your CI — so this can build and publish a " +
          "release of code nobody has read yet."
        : ""),
    confirmLabel: releasing
      ? "Apply, push and tag"
      : publishing
        ? "Apply and push"
        : "Apply the fixes",
    destructive: true,
  };
}
