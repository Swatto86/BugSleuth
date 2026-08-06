/**
 * Tests for the frontend's state rules.
 *
 * Plain assertions run by `node --test`, no framework. These cover the one
 * thing the UI is really responsible for: never letting a lane go uncovered
 * without saying so, and never disagreeing with the engine about what a
 * configuration means.
 */

import { strict as assert } from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = path.dirname(fileURLToPath(import.meta.url));

import {
  type ModelSetting,
  type Settings,
  LANES,
  LANE_TITLES,
  MAX_PASSES,
  MAX_PROVE_TOP,
  applyStatus,
  batchCount,
  boundedProveTop,
  canRun,
  joinId,
  passChoices,
  preset,
  splitId,
  toggleLane,
  uncoveredLanes,
  unitCount,
  vendorOf,
} from "./model.ts";

/**
 * A matrix row, with the fields every row really has.
 *
 * The tests used to write `{ id, lanes }` object literals, which type-check
 * only because nothing type-checked them: `tsconfig.json` excluded `*.test.ts`
 * and `node --experimental-strip-types` erases annotations without reading
 * them. Both are fixed, so the fixtures now have to be the real shape — and
 * where a test deliberately means an *older* shape, it says so with a cast.
 */
function row(id: string, lanes: string[]): ModelSetting {
  return { id, lanes, effort: "", passes: 1 };
}

test("every shipped preset covers every lane", () => {
  // A preset is exactly what someone picks when they have not thought about
  // lane coverage, so one that silently left a lane unswept would be the worst
  // possible default.
  for (const name of ["cheap", "balanced", "deep"] as const) {
    assert.deepEqual(
      uncoveredLanes(preset(name)),
      [],
      `${name} leaves a lane uncovered`,
    );
  }
});

test("an uncovered lane is reported", () => {
  // Derived from LANES rather than written out. A hand-copied list fails the
  // day a lane is added, for no defect, and the natural repair is to paste the
  // new name in — which is how a list stops meaning "all of them".
  const models = [row("sonnet", ["correctness"])];
  assert.deepEqual(
    uncoveredLanes(models),
    LANES.filter((lane) => lane !== "correctness"),
  );
});

test("a model with no id contributes nothing", () => {
  // A blank row is a half-typed entry, not a request to sweep.
  assert.equal(unitCount([row("  ", [...LANES])]), 0);
});

test("unknown lane names are ignored rather than counted", () => {
  assert.equal(unitCount([row("sonnet", ["correctness", "nonsense"])]), 1);
});

test("two models on one lane is two sweeps, because that is the point", () => {
  const models = [
    row("sonnet", ["correctness"]),
    row("codex:", ["correctness"]),
  ];
  assert.equal(unitCount(models), 2);
});

test("vendor parsing matches the engine, including bare and colon-containing names", () => {
  // If this drifts from the Rust side, the round estimate shown to the user is
  // simply wrong.
  assert.equal(vendorOf("sonnet"), "claude");
  assert.equal(vendorOf("claude:opus"), "claude");
  assert.equal(vendorOf("codex:gpt"), "codex");
  assert.equal(vendorOf("kilo:"), "kilo");
  assert.equal(vendorOf("anthropic:claude-opus-5"), "claude");
});

test("rounds are driven by the busiest vendor, not the total", () => {
  // The engine never runs two invocations of one vendor at once, so four sweeps
  // spread over two vendors is two rounds, not four.
  const models = [
    row("sonnet", ["correctness", "security"]),
    row("codex:", ["correctness", "security"]),
  ];
  assert.equal(unitCount(models), 4);
  assert.equal(batchCount(models), 2);
});

test("one vendor doing everything is one round per lane", () => {
  assert.equal(batchCount([row("sonnet", [...LANES])]), LANES.length);
});

test("an empty configuration needs no rounds", () => {
  assert.equal(batchCount([]), 0);
  assert.equal(unitCount([]), 0);
});

test("a run needs both a repository and at least one sweep", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    prove_top: 0,
    test_command: "",
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({ ...base, repo: "", models: preset("balanced") }),
    false,
  );
  assert.equal(canRun({ ...base, repo: "C:/x", models: [] }), false);
  assert.equal(
    canRun({ ...base, repo: "C:/x", models: preset("balanced") }),
    true,
  );
});

test("a blank row makes the configuration unrunnable, exactly like the engine", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    prove_top: 0,
    test_command: "",
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [row("sonnet", ["correctness"]), row("  ", [])],
    }),
    false,
    "Run must not be enabled with a half-typed row",
  );
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [row("sonnet", ["correctness", "nonsense"])],
    }),
    false,
    "an unknown lane must not be silently ignored",
  );
});

test("toggling a lane leaves other models untouched", () => {
  const models = [row("a", ["correctness"]), row("b", ["correctness"])];
  const next = toggleLane(models, 0, "security", true);
  assert.deepEqual(next[0]?.lanes.sort(), ["correctness", "security"]);
  assert.deepEqual(next[1]?.lanes, ["correctness"]);
  // The input must not have been mutated in place.
  assert.deepEqual(models[0]?.lanes, ["correctness"]);
});

test("toggling a lane off removes exactly it, and twice on is idempotent", () => {
  const models = [row("a", ["correctness", "security"])];
  assert.deepEqual(toggleLane(models, 0, "security", false)[0]?.lanes, [
    "correctness",
  ]);
  const twice = toggleLane(toggleLane(models, 0, "ux", true), 0, "ux", true);
  assert.equal(twice[0]?.lanes.filter((l) => l === "ux").length, 1);
});

test("lane titles match the engine's own names, including UX", () => {
  // "ux" capitalised naively becomes "Ux". The engine writes "UX", and one lane
  // named two ways in one product reads as carelessness.
  assert.equal(LANE_TITLES.ux, "UX");
  for (const lane of LANES) {
    assert.ok(LANE_TITLES[lane], `${lane} has no display title`);
  }
});

test("splitting a model spec and rejoining it settles on one spelling", () => {
  // Not byte-identity: `claude:opus` and `opus` are the same model to the
  // engine, and the round trip deliberately collapses them onto the bare form.
  // What must hold is that it settles — a second pass changes nothing, so a
  // model cannot drift each time the table is redrawn.
  for (const id of [
    "sonnet",
    "claude:opus",
    "codex:gpt-5.6-codex",
    "kilo:openrouter/z-ai/glm-4.6",
    "kilo:kilo/anthropic/claude-opus-5",
    "codex:",
    "kilo:",
  ]) {
    const once = joinId(splitId(id).vendor, splitId(id).model);
    const twice = joinId(splitId(once).vendor, splitId(once).model);
    assert.equal(twice, once, `${id} keeps changing: ${once} then ${twice}`);
    assert.equal(
      splitId(once).vendor,
      splitId(id).vendor,
      `${id} changed vendor`,
    );
  }
  // The one normalisation this performs, stated outright.
  assert.equal(
    joinId(splitId("claude:opus").vendor, splitId("claude:opus").model),
    "opus",
  );
  // Everything else is left exactly as written.
  assert.equal(
    joinId(
      splitId("kilo:openrouter/z-ai/glm-4.6").vendor,
      splitId("kilo:openrouter/z-ai/glm-4.6").model,
    ),
    "kilo:openrouter/z-ai/glm-4.6",
  );
});

test("a bare Claude model never gains a redundant prefix", () => {
  // `sonnet` and `claude:sonnet` are the same model to the engine but different
  // strings here, so a run configured with both would sweep twice and report as
  // two models. The UI must only ever produce the bare form.
  assert.equal(joinId("claude", "sonnet"), "sonnet");
  assert.equal(joinId("claude", "  sonnet  "), "sonnet");
});

test("splitId agrees with vendorOf on every shape", () => {
  // Two functions deciding vendor differently is how a row shows one provider
  // and bills another.
  for (const id of [
    "sonnet",
    "claude:opus",
    "codex:x",
    "kilo:a/b",
    "gpt:weird",
    "",
  ]) {
    assert.equal(splitId(id).vendor, vendorOf(id), `disagreement on ${id}`);
  }
});

test("an unknown prefix is treated as a Claude model, colon and all", () => {
  // `gpt:weird` is not a vendor prefix, so the whole string is the model — the
  // same reading the engine uses. Dropping the prefix would review a different
  // model than the one written down.
  assert.deepEqual(splitId("gpt:weird"), {
    vendor: "claude",
    model: "gpt:weird",
  });
});

test("a repeated pass is a whole extra sweep, and an extra round", () => {
  // Two passes of one model is two invocations of one vendor, which the engine
  // will never run at once — so it costs a round as well as a sweep.
  const models = [{ ...row("sonnet", ["correctness"]), passes: 2 }];
  assert.equal(unitCount(models), 2);
  assert.equal(batchCount(models), 2);
});

test("settings saved before passes existed still estimate as one pass each", () => {
  // Reading a missing field as NaN would put "NaN sweeps" in front of someone
  // whose only mistake was having used the app last week.
  //
  // Deliberately the *old* shape, with no `passes` field at all — the cast is
  // the point. Type-checking the tests turned this into `row(...)`, which
  // supplies `passes: 1`, and the test stopped exercising the missing-field
  // path entirely: it asserted 1 about an object that already said 1. A check
  // that can no longer fail, introduced by the very change that added the lane
  // which hunts them.
  const legacy = {
    id: "sonnet",
    lanes: ["correctness"],
  } as unknown as ModelSetting;
  assert.equal(unitCount([legacy]), 1);
});

test("a model listed twice with different passes counts like the engine: max, not sum", () => {
  // plan.rs enumerates (model, lane, effort, pass) tuples and drops exact
  // duplicates, so a second row for the same model+lane at 3 passes adds
  // passes 2 and 3 - its pass 1 collides with the first row's. Summing rows
  // shows "4 sweeps" for a run the engine executes as 3.
  const models = [
    { ...row("sonnet", ["correctness"]), passes: 1 },
    { ...row("sonnet", ["correctness"]), passes: 3 },
  ];
  assert.equal(unitCount(models), 3);
  assert.equal(batchCount(models), 3);
});

test("an identical duplicate row adds nothing, exactly like the engine", () => {
  const models = [
    row("sonnet", ["correctness"]),
    row("sonnet", ["correctness"]),
  ];
  assert.equal(unitCount(models), 1);
});

test("a proof count is clamped to the cap the field advertises", () => {
  // Typing (rather than spinning) past the max only marks the field invalid;
  // the value still reads straight through. 500 here is 500 model invocations
  // and 500 full test runs.
  assert.equal(boundedProveTop("500"), MAX_PROVE_TOP);
  assert.equal(boundedProveTop("-3"), 0);
  assert.equal(boundedProveTop("7"), 7);
});

test("a proof count is always an integer the backend can deserialize", () => {
  // prove_top is a usize in Rust. A float reached Tauri as JSON and failed to
  // deserialize, which stopped settings saving and runs starting - with a raw
  // deserialization error rather than anything a user could act on.
  assert.equal(boundedProveTop("1.5"), 1);
  assert.equal(boundedProveTop("0.9"), 0);
  assert.equal(boundedProveTop(""), 0);
  assert.equal(boundedProveTop("abc"), 0);
  for (const raw of ["500", "-3", "1.5", "", "abc", "2e3"]) {
    assert.ok(
      Number.isInteger(boundedProveTop(raw)),
      `${raw} produced a non-integer`,
    );
  }
});

test("the advertised cap and the enforced cap are the same number", () => {
  // The same rule written down on both sides of a boundary, with nothing making
  // them agree, is the defect class that produced this pair in the first place.
  const html = fs.readFileSync(path.join(here, "..", "index.html"), "utf8");
  const declared = /id="prove-top"[^>]*max="(\d+)"/.exec(html);
  assert.ok(declared, "the prove-top input no longer declares a max");
  assert.equal(Number(declared![1]), MAX_PROVE_TOP);
});

test("an apply that changed nothing does not claim to have applied anything", () => {
  // Observed in the real window: "Fixes applied — 0 files changed. Review the
  // diff." over a repository nobody had touched. The model had verified every
  // finding and correctly refused to edit code into agreement with a stale
  // report — but the status claimed an outcome it did not produce, and pointed
  // at a diff that does not exist.
  const nothing = applyStatus(true, 0);
  assert.ok(!nothing.includes("applied"), nothing);
  assert.ok(!nothing.includes("diff"), nothing);
  assert.match(nothing, /Nothing was changed/);

  // A real apply still says what happened, and singular is not "1 files".
  assert.match(applyStatus(true, 1), /1 file changed/);
  assert.match(applyStatus(true, 3), /3 files changed/);
  assert.match(applyStatus(false, 0), /failed/);
});

test("a stored pass count outside the picker presets remains visible", () => {
  // Rust caps `passes` at MAX_PASSES (25) and refuses the run above it, so a
  // stored value within the cap is a valid backend instruction. The selector
  // must offer it rather than silently showing 1 while Run still sends 5.
  assert.deepEqual(passChoices(5), [1, 2, 3, 5]);
  assert.ok(passChoices(5).includes(5));
  // Pre-passes settings (no field) and Rust's max(1) both normalise to one.
  assert.deepEqual(passChoices(undefined), [1, 2, 3]);
  assert.deepEqual(passChoices(0), [1, 2, 3]);
  // The usual values are unchanged and not duplicated.
  assert.deepEqual(passChoices(2), [1, 2, 3]);
});

test("a passes count above the backend cap is not runnable and is clamped in the picker", () => {
  const base = {
    scope: "",
    theme: "system" as const,
    prove_top: 0,
    test_command: "",
    reuse_completed: true,
    triage_model: "haiku",
    apply_model: "",
    apply_effort: "",
    push_after_apply: false,
  } satisfies Omit<Settings, "repo" | "models">;
  assert.equal(
    canRun({
      ...base,
      repo: "C:/x",
      models: [{ id: "sonnet", lanes: ["correctness"], effort: "", passes: 26 }],
    }),
    false,
    "Run must be disabled for a config the backend would refuse",
  );
  assert.ok(
    !passChoices(40).some((n) => n > MAX_PASSES),
    "the picker must not advertise a pass count above the backend cap",
  );
});
