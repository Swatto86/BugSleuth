/**
 * What the window holds before the saved file has been read.
 *
 * Split from `main.ts` at the hard line cap. It is a fallback, never a default
 * anyone chose: settings that fail to load leave these on screen, and `main.ts`
 * suppresses persistence until a real edit so a recoverable file is not
 * overwritten with them. Rust owns the shipped defaults; this only has to be a
 * complete, harmless object.
 */

import { DEFAULT_CLAUDE_SESSIONS, type Settings, preset } from "./model.ts";

/**
 * Which model re-grades severities. Cheapest available: the pass compares
 * summaries against each other, it does not review code again.
 */
export const TRIAGE_MODEL = "haiku";

export function startingSettings(): Settings {
  return {
    repo: "",
    scope: "",
    models: preset("balanced"),
    theme: "system",
    reuse_completed: true,
    triage_model: TRIAGE_MODEL,
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
    tag_release_after_push: false,
    claude_sessions: DEFAULT_CLAUDE_SESSIONS,
  };
}
