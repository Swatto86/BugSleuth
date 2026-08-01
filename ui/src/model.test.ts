/**
 * Tests for the frontend's state rules.
 *
 * Plain assertions run by `node --test`, no framework. These cover the one
 * thing the UI is really responsible for: never letting a lane go uncovered
 * without saying so, and never disagreeing with the engine about what a
 * configuration means.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import {
  LANES,
  batchCount,
  canRun,
  preset,
  toggleLane,
  uncoveredLanes,
  unitCount,
  vendorOf,
} from "./model.ts";

test("every shipped preset covers all four lanes", () => {
  // A preset is exactly what someone picks when they have not thought about
  // lane coverage, so one that silently left a lane unswept would be the worst
  // possible default.
  for (const name of ["cheap", "balanced", "deep"] as const) {
    assert.deepEqual(uncoveredLanes(preset(name)), [], `${name} leaves a lane uncovered`);
  }
});

test("an uncovered lane is reported", () => {
  const models = [{ id: "sonnet", lanes: ["correctness"] }];
  assert.deepEqual(uncoveredLanes(models), ["security", "contract", "ux"]);
});

test("a model with no id contributes nothing", () => {
  // A blank row is a half-typed entry, not a request to sweep.
  assert.equal(unitCount([{ id: "  ", lanes: [...LANES] }]), 0);
});

test("unknown lane names are ignored rather than counted", () => {
  assert.equal(unitCount([{ id: "sonnet", lanes: ["correctness", "nonsense"] }]), 1);
});

test("two models on one lane is two sweeps, because that is the point", () => {
  const models = [
    { id: "sonnet", lanes: ["correctness"] },
    { id: "codex:", lanes: ["correctness"] },
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
    { id: "sonnet", lanes: ["correctness", "security"] },
    { id: "codex:", lanes: ["correctness", "security"] },
  ];
  assert.equal(unitCount(models), 4);
  assert.equal(batchCount(models), 2);
});

test("one vendor doing everything is one round per lane", () => {
  assert.equal(batchCount([{ id: "sonnet", lanes: [...LANES] }]), 4);
});

test("an empty configuration needs no rounds", () => {
  assert.equal(batchCount([]), 0);
  assert.equal(unitCount([]), 0);
});

test("a run needs both a repository and at least one sweep", () => {
  const base = { scope: "", theme: "system" as const, prove_top: 0, test_command: "" };
  assert.equal(canRun({ ...base, repo: "", models: preset("balanced") }), false);
  assert.equal(canRun({ ...base, repo: "C:/x", models: [] }), false);
  assert.equal(canRun({ ...base, repo: "C:/x", models: preset("balanced") }), true);
});

test("toggling a lane leaves other models untouched", () => {
  const models = [
    { id: "a", lanes: ["correctness"] },
    { id: "b", lanes: ["correctness"] },
  ];
  const next = toggleLane(models, 0, "security", true);
  assert.deepEqual(next[0]?.lanes.sort(), ["correctness", "security"]);
  assert.deepEqual(next[1]?.lanes, ["correctness"]);
  // The input must not have been mutated in place.
  assert.deepEqual(models[0]?.lanes, ["correctness"]);
});

test("toggling a lane off removes exactly it, and twice on is idempotent", () => {
  const models = [{ id: "a", lanes: ["correctness", "security"] }];
  assert.deepEqual(toggleLane(models, 0, "security", false)[0]?.lanes, ["correctness"]);
  const twice = toggleLane(toggleLane(models, 0, "ux", true), 0, "ux", true);
  assert.equal(twice[0]?.lanes.filter((l) => l === "ux").length, 1);
});
