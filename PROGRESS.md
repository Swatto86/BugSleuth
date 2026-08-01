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
| M3 — Tauri UI | **Done.** Desktop app with tray, icon, dark/light, live progress |

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
  bugsleuth-engine/    composes the above: plan, run, merge, prove
  bugsleuth-cli/       the `bugsleuth` binary
src-tauri/             the desktop shell (thin: deserialize, call engine, serialize)
ui/                    vanilla TypeScript frontend, no framework
fixtures/seeded-repo/  a tiny crate with 6 known bugs (5 planted, 1 found by Codex)
fixtures/SEEDED.md     the answer key, kept OUTSIDE the fixture repo on purpose
eval/                  the M1 defect descriptions and what they measure
```

Five commands: `preflight`, `sweep`, `run`, `judge`, `prove`, plus a desktop
app (`cargo tauri dev` / `cargo tauri build`). See [README.md](README.md).

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

**Recall on real code is low. Two vendors, two misses.**

On the seeded fixture every vendor finds nearly everything, so it cannot
discriminate. On a real repository — Alder rolled back to before commit
`REDACTED` — the correctness lane returned:

| Vendor | Findings | Found the reverted bug? |
|---|---|---|
| Claude | 2 | **No** |
| Codex | 2 | **No** |
| Kilo | failed (silent non-zero exit) | — |

All four findings were real, anchored, and worth having — a silently dropped
attachment, a corrupted refresh token, and a timezone bug both vendors found
independently. But **the defect the experiment was designed around was found by
neither**, on a 27-file crate with the lane pointed straight at it.

That is the single most important number in the project. On a toy fixture recall
looks like 100%; on real code, for this one defect, it is 0 of 2. Everything else
rests on the sweep actually finding things, and one defect is far too small a
sample to conclude anything except that this needs measuring properly.

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

## Desktop app: verified end to end

**The full loop was driven through the real installed app**, by mouse and
keyboard, and observed:

1. Installed from the NSIS installer; Start-menu entry and desktop shortcut created.
2. Window appeared with all three providers detected live — claude 2.1.214,
   codex 0.146.0, kilo 7.4.16.
3. Repository set, models and lanes configured down to one sweep. Unchecking
   lanes raised the uncovered-lane warning immediately, with the Security,
   Contract and UX columns marked.
4. **A real review ran** — live progress streamed into the window
   (`Round 1/1: haiku × Correctness`, then `6 findings`).
5. The result landed on disk at
   `%APPDATA%\BugSleuth
uns\seeded-repo\correctness-haiku.json`: lane
   Correctness, model `claude:haiku`, status swept, **6 findings, 0 rejected**,
   every one anchored to a real line of the fixture.
6. Both themes render and switch.

That is the journey `~/.agents/tauri.md` asks for, asserting effects rather than
calls: the JSON on disk naming the model and carrying anchored findings cannot
be produced by anything but a real run.

**Two defects it found that nothing cheaper could:**

- Several colour pairs missed WCAG AA: light hint text at 3.4:1, dark hint text
  at 3.58:1, the light primary button's own label at 3.68:1, and control borders
  at 1.6:1 and 2.15:1 against the 3:1 a component boundary needs. None looked
  broken enough to file a bug against — at native resolution the light theme
  reads perfectly well — which is the point: an opinion cannot see a 3.4 that
  should be a 4.5. `theme.test.ts` now parses the CSS and measures every pair.
- The UI offered a "prove top N" control that `start_run` ignored entirely.

**Verified by observed effect on the packaged release binary:**

- Launches, window appears, ~25 MB resident.
- Closing the window hides it and the process stays alive (close-to-tray).
- The icon extracted *from the executable* is the BugSleuth mark, not a default.
- The lane matrix, presets, plan estimate and the uncovered-lane warning all
  behave correctly, checked in the live page.
- Both themes switch, and "match system" follows the OS rather than freezing.
- The Quit button reaches the backend over the IPC and is keyboard reachable.

**Still not driven:** the tray menu, and a click-through of Quit to process
exit. The `WebDriver` harness is committed but does not currently observe
anything — see `DECISIONS.md` 3.10 — so those were checked by construction and
by the in-app Quit reaching the backend over the IPC, not by clicking the tray.

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
- **Clustering was tuned on data that could not stress it.** The seeded fixture's
  findings all land within a line or two of each other, so a 3-line anchor window
  looked fine; on a real crate two vendors anchored the same defect 5 lines apart
  and it failed to merge. Now 10, with the real output committed as a second test
  corpus. Assume other thresholds have the same problem until real data has been
  through them.
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
