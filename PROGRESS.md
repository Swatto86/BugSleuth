# Progress

**Written for a session with zero context.** If you are resuming, read this file
and `DECISIONS.md`, then continue from "Next concrete step".

Last updated: 2026-07-31, unattended overnight session.

## Where we are

| Milestone | State |
|---|---|
| Read Eir and record findings | **Done** — `DECISIONS.md` part 1 |
| M0 — CLI harness, one provider, one lane, validated JSON with anchors | **Done and verified against a real model** |
| M1 — eval harness: can models produce a *failing test* for a bug they claim? | In progress |
| M2 — multi-model, multi-lane, judge | Not started. Gated on M1 |
| M3 — Tauri UI | **Out of scope tonight**, by instruction |

## Model invocation budget

Cap for the night: ~40. **Used so far: 1.**

| # | What | Model | Outcome |
|---|---|---|---|
| 1 | M0 sweep, correctness lane, `fixtures/seeded-repo` | sonnet | Success — 5 findings, 5 verified, 0 discarded, 8 turns |

## What exists

```
crates/
  bugsleuth-domain/    lanes, findings, ids. Types only, no I/O, no sibling deps
  bugsleuth-provider/  Claude Code CLI adapter + shared subprocess handling
  bugsleuth-verify/    anchor checking: does this finding quote real code?
  bugsleuth-cli/       the harness binary, `bugsleuth`
fixtures/
  seeded-repo/         a tiny crate with 5 deliberate bugs
  SEEDED.md            the answer key, kept OUTSIDE the fixture repo on purpose
scripts/
  verify.ps1           the full gate: fmt, clippy, tests, build, file sizes
  check-file-size.ps1  fails over 400 lines in any source file
```

Commands:

```bash
cargo run -p bugsleuth-cli -- preflight
```

```bash
cargo run -p bugsleuth-cli -- sweep --repo fixtures/seeded-repo --lane correctness --model sonnet --json-out eval-out/run.json
```

## Verified, with evidence

- `scripts/verify.ps1` passes: `cargo fmt`, `cargo clippy --all-targets -D warnings`, 33 tests, release build, file-size cap.
- **M0 end to end against a real model.** The correctness lane found all 5
  seeded defects in `fixtures/seeded-repo`, including the hardest one (a
  doc-comment-versus-code off-by-one). All 5 anchors verified against the real
  files. 0 discarded. See `NIGHT-REPORT.md` for the full output.

## Known gaps

- Only the Claude adapter exists. Codex and Kilo are M2.
- No persistence, no resume, no concurrency governor. All M2.
- No git worktree isolation yet — needed before any sweep is allowed to write,
  which M1's proof-carrying step requires.
- Timeout is per-invocation only; there is no overall run budget.

## Next concrete step

Build the M1 experiment: take a real repository, revert a known bug-fix commit,
and measure whether a model asked to prove a defect can write a test that
**fails against the buggy code and passes once fixed**. This is the experiment
that decides whether the proof-carrying design works at all. It runs in a git
worktree on a throwaway branch, never against a working tree.
