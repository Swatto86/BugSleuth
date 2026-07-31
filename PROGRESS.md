# Progress

**Written for a session with zero context.** If you are resuming, read this file
and `DECISIONS.md`, then continue from "Next concrete step".

Last updated: 1 August 2026, end of the unattended overnight session.

## Where we are

| Milestone | State |
|---|---|
| Read Eir and record findings | **Done** — `DECISIONS.md` part 1 |
| M0 — CLI harness, one provider, one lane, validated JSON with anchors | **Done, verified against a real model** |
| M1 — can a model produce a *failing test* for a bug it claims? | **Done. It can.** 4/4 correct across real and fabricated defects |
| M2 — multi-model, multi-lane, judge | **Partly done.** Second vendor (Codex) works and cross-vendor value is demonstrated. **The judge is not built**, so the M2 kill gate has not been run |
| M3 — Tauri UI | **Out of scope**, by instruction |

## Model invocation budget

Cap for the night was ~40. **Used: 7.** No rate limit was hit.

| # | What | Model | Outcome |
|---|---|---|---|
| 1 | M0 sweep, correctness, seeded fixture | claude sonnet | 5 findings, 5 verified, 0 discarded, 8 turns |
| 2 | Prove real Alder defect | claude sonnet | **PROVED**, 7 turns |
| 3 | Prove fabricated Alder defect | claude sonnet | Correctly refused, 13 turns |
| 4 | Prove real fixture defect | claude sonnet | **PROVED**, 5 turns |
| 5 | Prove fabricated fixture defect | claude sonnet | Correctly refused, 5 turns |
| 6 | Codex sweep | `gpt-5.6-codex` | Failed instantly: that model id is not available on a ChatGPT account. Cost nothing |
| 7 | Codex sweep, correctness, seeded fixture | codex default | 9 findings, 9 verified, 0 discarded |

## What exists

```
crates/
  bugsleuth-domain/    lanes, findings, proof verdicts. Types only, no I/O
  bugsleuth-provider/  Claude and Codex CLI adapters + shared subprocess handling
  bugsleuth-verify/    anchor checking, git worktrees, test execution
  bugsleuth-cli/       the harness binary, `bugsleuth`
fixtures/
  seeded-repo/         a tiny crate with 6 known bugs (5 planted, 1 found by Codex)
  SEEDED.md            the answer key, kept OUTSIDE the fixture repo on purpose
eval/                  the M1 defect descriptions and what they measure
scripts/
  verify.ps1           the full gate: fmt, clippy, tests, build, file sizes
```

Commands:

```bash
cargo run -p bugsleuth-cli -- preflight
```

```bash
cargo run -p bugsleuth-cli -- sweep --repo fixtures/seeded-repo --lane correctness --model sonnet
```

```bash
cargo run -p bugsleuth-cli -- sweep --repo fixtures/seeded-repo --lane correctness --model codex:
```

```bash
cargo run -p bugsleuth-cli -- prove --repo <git repo> --defect-file <file> --test-command "cargo test"
```

`--model` takes `vendor:model`. A bare name means Claude. `codex:` with nothing
after it means Codex's own default model — note that `gpt-5.6-codex` is **not**
usable on a ChatGPT subscription.

## Verified, with evidence

- `scripts/verify.ps1` passes: fmt, clippy with warnings as errors, 66 tests,
  release build, 400-line file cap.
- **M0**: the correctness lane found all 5 planted defects in the fixture, all
  anchored to real code, none discarded.
- **M1**: 2 real defects proved with genuine failing tests; 2 fabricated defects
  correctly refused. For the real Alder defect the model's test was also
  confirmed to **pass** once the historical fix was applied — so it detects the
  bug rather than failing for an unrelated reason.
- **Cross-vendor**: same lane, same repo — Claude 5 findings, Codex 9. Codex
  found a real defect neither Claude nor the fixture author had noticed;
  confirmed by running the function.

## Known gaps

- **No judge.** This is the main thing standing between here and finishing M2.
- **The M2 kill gate has not been run** — it needs the judge.
- Kilo, the third vendor, is not wired up.
- Codex can sweep but cannot prove; the proof path is Claude-only.
- No persistence, no resume, no concurrency governor. A run that dies restarts.
- Detection rate on a real codebase is unmeasured. M0 measured plumbing on a toy
  fixture, not recall on real code.
- The UX, Security and Contract lanes have mandates written but have never been
  run against anything.

## Next concrete step

**Build the within-lane judge and run the M2 kill gate.**

The input it needs now exists: two vendors produce overlapping findings on the
same code (see `eval-out/m0-correctness-sonnet.json` and
`eval-out/m2-correctness-codex.json` for a real, already-paid-for pair).

The judge should merge duplicates, count how many models independently found
each defect, and rank. Strip which model reported what before judging — models
favour their own family's output. Put the judge on the BYOD API key rather than
a subscription CLI: it needs strict structured output, has no repo access, and
is pure text in, text out.

Then the gate: **if the judge's top-10 is no better than the best single model's
top-10, stop building.** On the current evidence Codex alone found a superset of
Claude's findings on this fixture, so a judge must be shown to add something
beyond "use the better model", or the premise is in trouble. That is the honest
framing of the test, and the fixture is probably too small to settle it — a
larger repository is likely needed for the gate to mean anything.
