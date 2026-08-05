/**
 * Tests for the wording.
 *
 * These look trivial and are not. The whole product rests on a reader being
 * able to tell "we looked and found nothing" from "we never looked", and that
 * distinction is made here, in a few lines of string formatting. Getting it
 * wrong would not crash anything — it would quietly tell someone their code is
 * fine.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { describe, listOf, type RunEvent } from "./format.ts";

test("a sweep that found nothing is not confusable with one that never ran", () => {
  const found: RunEvent = {
    kind: "sweep_finished",
    model: "claude:sonnet",
    lane: "Security",
    findings: 0,
    swept: true,
    reason: "",
  };
  const never: RunEvent = {
    kind: "sweep_finished",
    model: "claude:sonnet",
    lane: "Security",
    findings: 0,
    swept: false,
    reason: "the CLI timed out",
  };

  const foundText = describe(found);
  const neverText = describe(never);

  assert.ok(foundText.includes("0 findings"), foundText);
  assert.ok(!foundText.includes("NOT SWEPT"), foundText);

  assert.ok(neverText.includes("NOT SWEPT"), neverText);
  assert.ok(neverText.includes("the CLI timed out"), neverText);
  // The failing case must never render as a count, which reads as "looked".
  assert.ok(!neverText.includes("0 findings"), neverText);
});

test("a single finding is not reported as '1 findings'", () => {
  const one: RunEvent = {
    kind: "sweep_finished",
    model: "codex:",
    lane: "UX",
    findings: 1,
    swept: true,
    reason: "",
  };
  assert.ok(describe(one).includes("1 finding"), describe(one));
  assert.ok(!describe(one).includes("1 findings"), describe(one));
});

test("a round names what is about to run and where it sits in the sequence", () => {
  const batch: RunEvent = {
    kind: "batch_started",
    index: 2,
    total: 3,
    units: ["sonnet x Correctness", "codex: x Security"],
  };
  const text = describe(batch);
  assert.ok(text.includes("2/3"), text);
  assert.ok(text.includes("sonnet x Correctness"), text);
  assert.ok(text.includes("codex: x Security"), text);
});

test("a reused sweep says it was reused rather than pretending to be fresh", () => {
  const reused: RunEvent = {
    kind: "reused",
    model: "sonnet",
    lane: "Contract",
  };
  const text = describe(reused);
  assert.ok(text.includes("reused"), text);
  assert.ok(text.includes("Contract"), text);
});

test("names are joined the way English does", () => {
  assert.equal(listOf([]), "");
  assert.equal(listOf(["Security"]), "Security");
  assert.equal(listOf(["Security", "UX"]), "Security and UX");
  assert.equal(
    listOf(["Security", "Contract", "UX"]),
    "Security, Contract and UX",
  );
});
