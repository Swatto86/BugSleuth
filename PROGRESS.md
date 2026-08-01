# Progress

**Written for a session with zero context.** If you are resuming, read this file
and `DECISIONS.md`, then continue from "Next concrete step".

Last updated: 1 August 2026.

## Where we are

| Milestone | State |
|---|---|
| Read Eir and record findings | **Done** — `DECISIONS.md` part 1 |
| M0 — CLI harness, validated JSON with anchors | **Done, verified against real models** |
| M1 — can a model produce a *failing test* for a bug it claims? | **Done. It can.** 4/4 correct across real and fabricated defects |
| M2 — multi-model, multi-lane, judge | **Done.** Three vendors, deterministic judge, `run` orchestrates the whole product. Kill gate passed on weak evidence |
| M3 — Tauri UI | **Not started**, deliberately |

## Model invocation budget

**Used: 16.** No rate limit was hit, but one Kilo sweep died with the silent
non-zero exit these CLIs use for an overload when three ran concurrently — which
is why sweeps are now batched one-per-vendor.

## What exists

```
crates/
  bugsleuth-domain/    lanes, findings, proof verdicts. Types only, no I/O
  bugsleuth-provider/  Claude, Codex and Kilo adapters + shared subprocess handling
  bugsleuth-verify/    anchor checking, git worktrees, test execution
  bugsleuth-judge/     clustering, agreement counting, ranking
  bugsleuth-cli/       the `bugsleuth` binary
fixtures/seeded-repo/  a tiny crate with 6 known bugs (5 planted, 1 found by Codex)
fixtures/SEEDED.md     the answer key, kept OUTSIDE the fixture repo on purpose
eval/                  the M1 defect descriptions and what they measure
```

Five commands: `preflight`, `sweep`, `run`, `judge`, `prove`. See
[README.md](README.md).

## Verified, with evidence

- `scripts/verify.ps1` passes: fmt, clippy with warnings as errors, ~125 tests,
  release build, 400-line file cap.
- **M1**: 2 real defects proved with genuine failing tests; 2 fabricated defects
  correctly refused. For the real Alder defect the model's test was also
  confirmed to **pass** once the historical fix was applied.
- **Three vendors work** and are detected by `preflight`.
- **Judge**: on hand-labelled real cross-vendor output (committed as test data),
  23 findings merge to 11 distinct defects with correct separation of two
  different defects reported one line apart.
- **Kill gate passed, weakly.** Every vendor missed something another found;
  running only the best single vendor costs 2 of 11 defects. But see below.

## The most important open question

**Recall on real code is low, and the one measurement I have is a miss.**

On the seeded fixture every vendor finds nearly everything, so it cannot
discriminate. On a real repository — Alder rolled back to before commit
`REDACTED` — the Claude correctness sweep returned **2 findings, and neither was
the reverted bug.** Both findings were real and anchored, but the defect the
experiment was designed around was not found.

This is the single most important number in the project and it is currently
based on one lane and one vendor. Everything else rests on the sweep actually
finding things.

### A design gap that showed up with it

The reverted bug is a *silent UI failure* — an image is stripped and no banner
offers to load it — but it lives in backend Rust (`crates/infrastructure/src/html.rs`).

- The **UX lane** is the one whose mandate covers "a failure that is swallowed so
  the user sees nothing", but its file filter only matches frontend extensions,
  so it would reject a finding in a `.rs` file outright.
- The **correctness lane** can see the file but its mandate does not point at
  user-visible consequences.

So this defect currently falls between lanes. That is a real design question, not
a bug to quietly patch: widening the UX lane to all files risks exactly the
aesthetic-opinion pollution the file filter exists to prevent. **This needs your
decision.**

## Operational facts learned by running it on a real repository

- **Vendors differ enormously in speed.** On the same crate with the same brief,
  Claude finished in about five minutes. Codex was still working at twenty-five
  and was killed by the timeout. Kilo failed outright. Budget for the slow one.
- **The default timeout was too short**, now raised to 45 minutes for `run`.
- **Three CLIs started at once is too many.** One died with the silent non-zero
  exit these tools use for an overload, which is why sweeps are now batched
  one-per-vendor.
- **The failure paths all behaved correctly**, which is the reassuring part: the
  timeout killed its child with no orphan left, and both failures were reported
  as `NOT SWEPT` with the reason and a non-zero exit rather than as clean lanes.
  A tool that had returned "no findings" for either would have been dangerous.

## Known gaps

- **Kilo cannot prove.** Claude and Codex both can. Kilo is refused with a stated
  reason: it cannot be given an output schema to enforce, so its account of what
  it did cannot be relied on.
- **Only the correctness lane has ever been run.** Security, Contract and UX have
  written mandates and have never been pointed at anything.
- **No cross-lane severity pass.** A multi-lane report warns that severities are
  not comparable across lanes, but does not yet rank within each lane separately.
  Deferred until more than one lane has actually been run, so it can be designed
  against real output.
- **Same model, same input, different answers.** Two identical Claude sweeps of
  the fixture returned 5 findings and 7 findings. Run-to-run variance is real and
  unquantified.
- **Compound findings do not merge.** When one model bundles two defects into one
  finding it stays separate from both single-defect counterparts. Documented at
  the threshold in `crates/bugsleuth-judge/src/cluster.rs`.

## Next concrete step

**Measure recall properly on real code.** Revert several known bug-fix commits in
a real repository and count how many each vendor finds, per lane. Cheap now that
`run` and `--resume` exist:

```bash
cargo run -p bugsleuth-cli -- run --repo <clone> --config bugsleuth.example.json --out-dir runs/ --resume
```

Two things to fix in the experiment design first, both learned the hard way:

1. **Check every planted defect is actually absent from the code**, and every
   fabricated one actually false. A draft control in the M1 eval turned out to be
   *true* on inspection and would have measured the opposite of its intent.
2. **Point the right lane at it.** The banner bug is a UX-mandate defect in a
   backend file — see the design gap above.
