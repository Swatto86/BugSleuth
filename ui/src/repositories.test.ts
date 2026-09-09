import { strict as assert } from "node:assert";
import { test } from "node:test";
import { clonePlan, repositories, setRepositories } from "./repositories.ts";
import { advance, initialProgress, progressSummary } from "./scan-progress.ts";
import type { Settings } from "./model";

test("one repository list preserves legacy settings and validates clone destinations", () => {
  const settings = { repo: "first", additional_repos: ["second"] } as Settings;
  assert.deepEqual(repositories(settings), ["first", "second"]);
  setRepositories(settings, ["second", " third ", "second"]);
  assert.equal(settings.repo, "second");
  assert.deepEqual(settings.additional_repos, ["third"]);
  assert.deepEqual(
    clonePlan("https://host/a.git\ngit@host:owner/b.git", "", 2),
    [
      { source: "https://host/a.git", name: "a" },
      { source: "git@host:owner/b.git", name: "b" },
    ],
  );
  assert.throws(
    () => clonePlan("https://host/a.git\nhttps://elsewhere/a.git", "", 0),
    /same folder/,
  );
  assert.throws(
    () => clonePlan("https://host/a.git\nhttps://host/b.git", "override", 0),
    /override/,
  );
  assert.throws(() => clonePlan("https://host/a.git", "", 16), /16/);
  assert.throws(
    () => clonePlan("https://host/a.git", "../unsafe", 0),
    /safe folder/,
  );
});

test("scan progress separates completed zero findings, failures and reused reviews", () => {
  const progress = initialProgress(3);
  advance(progress, {
    kind: "batch_started",
    index: 1,
    total: 1,
    units: ["opus x Security"],
  });
  assert.match(progress.batch, /queued/);
  advance(progress, {
    kind: "sweep_finished",
    model: "opus",
    lane: "Security",
    findings: 0,
    swept: true,
    reason: "",
  });
  advance(progress, {
    kind: "sweep_finished",
    model: "cursor:auto",
    lane: "Security",
    findings: 0,
    swept: false,
    reason: "quota",
  });
  advance(progress, { kind: "reused", model: "opus", lane: "UX" });
  assert.equal(
    progressSummary(progress),
    "3/3 reviews returned · 1 completed · 1 reused · 1 not reviewed",
  );
  assert.match(progress.results[0]!, /Completed · 0 findings/);
  assert.match(progress.results[1]!, /Not reviewed: quota/);
  assert.match(progress.batch, /merging/);
});
