# Progress

**Written for a session with zero context.** If you are resuming, read this file
and `DECISIONS.md`, then continue from "Next concrete step".

Last updated: 1 August 2026 (afternoon).

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

### Measured properly: 1 of 3, and no agreement at all

The experiment the note above asked for, run and graded.

**Setup.** A scratch clone of Alder at the commit before the first of six
documented fix commits, so all six defects are present naturally — no reverts,
no conflicts. `PLAN.md` and `BUGSWEEP.md` were removed from that tree first:
they enumerate the very defects being measured, and leaving them in would have
measured reading comprehension. Three vendors, one lane, scope narrowed to the
three files holding three of the known defects — so every vendor was pointed
straight at them and at each other.

**Graded independently rather than by me.** Two judges per defect, each told to
answer "not found" when unsure, and told explicitly that same file, same
function, or symptom-without-cause all count as *not* found. Both judges agreed
on every one of the three.

| Known defect | Found? |
|---|---|
| #1 rotated refresh token destroyed by non-atomic keyring write | **No** |
| #4 `hasAttachments`-only delta silently dropped | **No** |
| #5 load-images banner misses non-canonical markup | **Yes** — Codex |

**Recall: 1 of 3**, with every vendor reading the exact files containing them.

Defect #1 is the one to look at. Claude reported a defect in the *same file and
the same function area* — a swallowed keyring error leaving orphaned chunks.
Both judges called it out as a different fault, "the mirror image of the actual
defect", and noted that fixing it would leave the delete-then-write ordering
untouched. Graded by hand it would very likely have been scored a hit. That is
the strongest argument in this project for not marking your own homework.

**Agreement: 0 of 7.** Every cross-vendor pair was judged, and not one pair
described the same defect. Five findings, five distinct defects.

That result kills the explanation offered earlier in this file. The claim was
that agreement fails because independent agents exploring a large repository
rarely read the same files. Co-location was then forced — one lane, three
files — and agreement did not appear. So the explanation was at best
incomplete: the vendors do not merely look in different places, they see
different things in the same place.

**What this does and does not establish.** Three defects and seven pairs is a
small sample from one repository. It does not establish a recall *rate*. What
it does establish is that neither the coverage explanation nor the merge
threshold accounts for the missing corroboration, and that a defect sitting in
a file a vendor is actively reading can still be missed by all three.

### Then: 3 of 3, after naming the two classes that were missed

Both misses had the same shape. Every defect class the correctness brief listed
was **visible in code that is present** — a wrong comparison, an unhandled case,
a leak. Defect #1 is a *deleting before committing* ordering, and defect #4 is a
field that arrives and is never acted on. Neither has a wrong line to spot. They
are found by asking what is missing, and nothing was asking.

So the brief now names them, in those words, with instructions to go looking:
check every sequence that replaces stored state for a destroy-before-write
window, and compare each incoming field against what is actually acted on.

The identical experiment was then re-run — same repository, same three files,
same three vendors, same settings, nothing changed but the brief:

| Known defect | Found before | Found after |
|---|---|---|
| #1 rotated refresh token destroyed by non-atomic keyring write | No | **Yes — all three vendors** |
| #4 `hasAttachments`-only delta silently dropped | No | **Yes — Codex and Claude** |
| #5 load-images banner misses non-canonical markup | Yes — Codex | **Yes — Codex** |

**Recall: 3 of 3, up from 1 of 3.** The fix plans match the real fix commits
closely: Claude's plan for #4 names the domain enum, the graph classification,
the sync consumer and the store — which is exactly the four-file change the
actual fix made.

This is one run per condition, so model-to-model variation contributes something.
But defect #1 went from missed by every vendor to found by every vendor, and the
variance measurement below says *what* gets found is the stable part of a sweep.
A coincidence of that size is not the likeliest reading.

**And agreement finally appeared.** Three vendors on defect #1, two on defect #4
— the first real corroboration measured in this project, against 0 of 7 before.
The earlier conclusion that vendors "see different things in the same place" was
too strong. What is true is narrower: left to characterise a defect however they
like, they describe it differently; told exactly which class to look for, they
converge on it. Corroboration is not absent, it is *manufacturable* — by the
brief. Not enough data yet to put agreement back into the ranking, but the
earlier reading needs correcting rather than defending.

### A design gap that showed up with it

The reverted bug is a *silent UI failure* — an image is stripped and no banner
offers to load it — but it lives in backend Rust (`crates/infrastructure/src/html.rs`).

- The **UX lane** is the one whose mandate covers "a failure that is swallowed so
  the user sees nothing", but its file filter only matches frontend extensions,
  so it would reject a finding in a `.rs` file outright.
- The **correctness lane** can see the file but its mandate does not point at
  user-visible consequences.

So this defect fell between lanes, reachable by neither.

**Resolved, on evidence.** The filter existed to stop the UX lane filing "no
loading state" against a backend module — and across every run on disk, three
full multi-lane sweeps of a real repository with thirteen-plus UX findings, it
rejected exactly nothing. Its guard had never once fired; its false negative was
demonstrated. A check that cannot catch what it was built for and does reject
what it should not is worse than no check, so it is gone.

The mandate carries the guard alone, which is what was doing the work anyway:
every UX finding across those runs was behavioural — missing confirmations,
unannounced status changes, focus traps — with no aesthetic opinions among them.
It now also states that a user-visible failure counts wherever it is
implemented, and requires each finding to name both the code and the symptom the
user experiences. Naming the symptom is the real discriminator; the file
extension never was.

If aesthetic pollution ever does appear, the answer is a check on the finding's
content, not on its path. See `DECISIONS.md`.

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
   `%APPDATA%\BugSleuth\runs\seeded-repo\correctness-haiku.json`: lane
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

**The tray was driven too**, by clicking it:

- Closing the window hid it and left the process alive at 28 MB.
- The tray icon carries a "BugSleuth" tooltip; left-clicking restored the window
  with its state intact.
- Right-click showed **Open BugSleuth** and **Quit**.
- **Quit exited cleanly** — process gone, no orphans, settings preserved.

So every desktop behaviour is now verified by observed effect. The full journey
is written up in [RUNBOOK.md](RUNBOOK.md).

**Still outstanding:** the WebDriver harness does not observe anything — see
`DECISIONS.md` 3.10. It is committed and correct in shape but must not be
treated as passing evidence; the runbook is the acceptance test until it works.

## What changed after the first real run

Running the tool on a real repository — Alder, 35 source files, 956 KB —
found more defects in BugSleuth than the fixture ever had. In rough order of
how much they mattered:

**Findings now say how to fix themselves.** Every finding carries a fix plan:
the root-cause approach, per-file edits naming the function rather than a line
number, the test that must fail before and pass after with its exact command,
and the callers that were checked. It is written for a named reader — a smaller
model running locally that has not read the review and cannot ask a question —
because a fix written for someone who already understands the bug is a
different document. The merged report grew a "Fixing these" section of
self-contained work orders; before this it showed title, location and agreement
only, which nobody could act on without opening the code.

**Kilo failed every sweep of a real repository, three problems deep.** The
report said "produced no diagnostic output" because the adapter only read
stderr, and Kilo streams its errors as JSON on stdout. Underneath that, Kilo
was loading the reviewed repository's own `CONTEXT.md` — 165 KB of it — and
running out of context before reading any code. Underneath *that*, its default
model simply cannot hold a repository this size. All three are now addressed
and Kilo sweeps Alder successfully.

The middle one is the important one, and it is not about size. Claude and Codex
already refuse the reviewed repository's instructions by flag; Kilo has no such
flag, so its throwaway worktree is now stripped of them. A repository could
otherwise have told its own reviewer what not to report.

**The run's own report was being thrown away.** The last progress event and the
finished event are emitted back to back, and the progress event arrived second
— painting a log of what had just happened over twenty ranked defects.

**The app was opening console windows.** Every provider CLI, the test runner and
every `git` call is a console program, and Windows gives one launched from a
windowed process a console of its own. A run is a dozen of them.

**Choosing a model is now a menu, not a memory test.** Provider, model and
effort per row. Kilo's 638 models are fetched live and each suggestion carries
the account it spends from — including the handful that carry a `kilo/` prefix
but bill to a plan you bought from the provider directly, which no amount of
reading the id would tell you. Effort levels come from each model's own
metadata, because they are not uniform: of those 638, 254 accept no effort at
all and some accept `instant`/`thinking` rather than a ladder.

**One thing that was not a bug.** Twice I reported that the app was losing its
settings. It was not: my shell reads a redirected filesystem, so the file I
could see was never the file the app writes. Typing a value, killing the
process and relaunching restores it. What came out of the hunt is worth having
anyway — a failed save or load now says so in the status bar instead of being
swallowed.

### The re-run, with everything working

Twelve sweeps, three vendors, four lanes, driven from the app.

| | Correctness | Security | Contract | UX |
|---|---|---|---|---|
| claude:sonnet | 9 | failed | failed | 5 |
| codex: | 2 | 1 | 0 | 1 |
| kilo (kimi-for-coding) | failed | failed | failed | **7** |

**7 of 12 swept, 25 findings, and every one of the 25 carries a usable fix
plan** — approach, edits naming a symbol, a verification command and checked
callers — from all three vendors.

Two things worth keeping in view.

**The failures are informative rather than silent, which is the design working.**
Claude's two were a bare non-zero exit; that is what exposed the same
stdout-versus-stderr gap Kilo had, now fixed for both. Kilo's three were schema
drift — on the Security lane it said outright *"Before I emit the final JSON, I
need the expected schema"*, having been given it. That is what the worked
example in the brief now addresses, and the fix is verified: the Correctness
lane that failed with `missing field 'title'` sweeps clean afterwards.

**Overlap is almost nil**, and the reason is now measured rather than guessed —
see "Measured properly" above. It is not the merge threshold and it is not
file coverage.

### Why the vendors barely agree

Measured rather than argued, over every same-file cross-vendor pair in three
corpora. Two causes, and only one of them was a bug.

**The judge's similarity measure was wrong, and had been all along.** It used
Jaccard — shared words over the *union* — with a doc comment saying set overlap
is not distorted when the same defect is described at very different lengths.
That is precisely Jaccard's weakness. A short description fully contained in a
long one is bounded by the ratio of their lengths: 22 words inside 77 cannot
exceed 0.29 however perfectly they agree, and the merge threshold was 0.20.

Two vendors reported the same missing-`aria-live` defect on the same line of
the same file, in 22 words and 77. It scored 0.125 and was reported twice.

It is now the overlap coefficient — shared words over the *smaller* set — which
is immune to length asymmetry. Threshold re-measured on all three corpora at
0.35. On the Alder run that takes 25 findings to 23 distinct defects with
**two** cross-vendor agreements instead of one, both at the top of the ranking.
The fixture was quietly losing merges to this too, at 0.12 and 0.16.

**But the dominant cause is not the judge — it is that the vendors never read
the same code.** Across the whole run, exactly **two files** received findings
from more than one vendor. In the Correctness lane Claude reported in six files
and Codex in one, overlapping on a single file; in UX, Claude two, Codex one and
Kilo three, again overlapping on one.

That reframes the question. Agreement requires co-location, and independent
agents exploring a 956 KB repository mostly do not choose the same files. So
"found by N models" cannot rank a broad sweep — not because the models disagree,
but because they are rarely looking at the same thing. It would work on a
*narrow* scope where every vendor must read the same code, which is a different
product from the one being built.

The honest position: cross-vendor review is demonstrably buying **coverage** —
every vendor found real defects the others missed — and is not yet buying
**corroboration**. Whether corroboration is reachable by scoping sweeps
narrowly, or should be dropped as a ranking signal, is undecided and needs a
deliberate experiment rather than another threshold.

## Precision: 78%, measured

The claim the whole design rests on is that a confident wrong finding is worse
than no finding. Recall had been measured; precision never had.

Every surviving finding from two real runs of Alder — the four-model UX sweep
and the three-vendor correctness experiment, 18 in total — was given to **three
independent sceptics each**, told to assume it was wrong, told to answer "not
real" when unsure, and told that a style preference or a hypothetical with no
concrete trigger does not count.

**14 of 18 are real. All 14 unanimous. The 4 refuted were unanimous too** — no
finding split its judges. Whatever else is true of these models, they are not
producing a spread of half-defensible claims; findings tend to be clearly right
or clearly wrong.

That is a usable number. Roughly one finding in five is not worth acting on,
which is why the prompt tells the implementer to verify before editing rather
than to trust the list.

### Re-measured after the mandate work: 12 of 13 (92%) on the UX corpus

The thirteen UX findings were graded again, one strict judge per finding, each
required to read the real code and to search the whole source tree for the
handling a finding claims is absent before believing the claim. Twelve are real.

The one false positive has a teachable shape: the reviewer said folder actions
were reachable only by right-click, but the folder rows are native `<button>`
elements — the platform itself gives those Enter/Space activation and a
Shift+F10 path to the very `contextmenu` listener in question. The reviewer got
the sibling finding *right* (message rows are plain `<div>`s, genuinely
unreachable); it never checked which kind of element this one targeted. The UX
mandate now instructs exactly that check, with a test, the same route that took
correctness recall from 1 of 3 to 3 of 3.

### A grading mistake worth keeping: three correct findings condemned as fabricated

The first grading pass returned 10 of 13 and called three findings fabrications
— quoted code that "did not exist", functions "invented", a comparison built on
a comment "not present in the file". Every one of those elements exists,
verbatim, in the repository the sweep actually reviewed. The graders had been
pointed at a *different checkout* of the same project, and they did their job
precisely: they reported exactly which claims did not hold on the tree they
were given.

Nothing in the report said which tree the sweep saw, so nothing could catch
this. Now something does: every sweep records the commit it reviewed (old
reports without one still load), and merging sweeps that reviewed different
commits prints a warning naming them. The false-fabrication episode is the
argument: a finding is a claim about one exact version of the code, and a
report that omits the version invites exactly this mis-grade.

### Severity was wrong 6 times in 14, in both directions

Worse than the rate is the direction. Two of the most serious defects were
*under*-ranked — a missing `aria-live` region covering some fifty call sites,
and a sign-in path that fails permanently with no workaround — while two
keyboard-convenience gaps were over-ranked. "Worst first" was not holding.

The cause turned out to be embarrassing and easy: **the schema's `severity`
field was a bare enum with no description**. Four words, no definition, no
rubric. The models were assigning severity by instinct because nothing had ever
told them what the levels meant.

It now carries the same consequence-based rubric the judges were given — critical
is data loss or an unusable app, high is a common action failing or an
accessibility barrier with no way round it, medium has a workaround, low is
minor — and the test asserts every level is defined. The fix prompt also now says
outright that the ordering is a guide rather than an authority, and that if a
defect looks worse than its label it should be treated as worse.

## Known gaps

- **Kilo cannot prove.** Claude and Codex both can. Kilo is refused with a stated
  reason: it cannot be given an output schema to enforce, so its account of what
  it did cannot be relied on.
- ~~Proving has never been run from the app.~~ **Done.** Both branches observed:
  it refuses a non-git target with a stated reason, and on a git-backed copy of
  the fixture it wrote a test that fails because of the defect, confirmed every
  previously passing test still passes, and reported "1 proved, 0 not" while
  saying outright that the other five were *not attempted*, which is not the
  same as not provable.
- **Nothing merges on real code, and agreement has been retired because of it.**
  Three attempts to produce cross-vendor agreement, each under better conditions
  than the last: a full multi-lane run gave 2 merges in 23 defects; an
  experiment that forced three vendors onto the same three files gave 0 of 7
  pairs, confirmed by independent judges; a four-model sweep of the richest lane
  gave 13 findings and 13 distinct defects. Ranking on a signal that is almost
  always 1-versus-1 implied a confidence the data does not support, so it no
  longer orders anything. It is still counted and shown, relabelled from
  "Confidence" to "Reported by".
- ~~A run that dies loses every sweep before it in the app.~~ **Done.** The
  desktop app now reuses completed sweeps by default — the opposite of the
  command line, because the case that actually happens is "it died at nine of
  twelve and I pressed Run again". Failed sweeps are still retried.
- ~~A malformed Kilo reply loses the whole sweep.~~ **Done.** Three sweeps died
  that way in one day, each after the expensive part was finished. A malformed
  reply now gets one cheap reshape attempt that reads no code and is told to
  invent nothing; the anchor check still runs on whatever comes back.
- ~~Severity is now the only ranking key, and it is self-assigned.~~ **Now
  graded once more with the whole report in view.** Retiring agreement left
  severity ordering everything, and severity was whatever the model that found
  the defect happened to call it — graded by hand against a real run, 6 of 14
  were wrong, in both directions.

  The reason is structural rather than any model being bad at it. A sweep grades
  each defect in isolation with no reference class, so a lane whose worst find is
  an unhandled edge case still crowns one of them "high". "Worst first" is a
  claim about a total order, and a total order can only come from comparison. So
  one cheap call now grades every merged defect together against a written
  rubric — the same rubric the sweeps were given, from a single constant, so the
  two cannot drift apart.

  **What it actually buys, measured on 13 real defects, three runs: 1, 2 and 3
  changes.** Small, and worth being plain about. What it changed is more
  interesting than how much: the moves brought *sibling defects into agreement* —
  two accounts of the same missing confirmation dialog that clustering had failed
  to merge were graded high and medium, and triage made them agree. That is
  precisely the failure isolation causes.

  It never invents, removes or re-words a defect, a verdict for an id that was
  never offered is dropped, and a pass that fails leaves every original grade
  alone and says so in the report. Each defect that moved is printed with what it
  moved from.

- **Getting there cost four wrong diagnoses, and the last one was mine.** The
  pass kept dying with `error_max_turns`. First reading: too few turns — raised
  12 to 30, still died. Second: the file reads were burning the budget — took
  the tools away, still died. Third: the reviewed repo's own agent instructions
  were leaking in, as they once did for Kilo — disproved by running the CLI by
  hand from that directory, which worked fine.

  The actual cause was that a `cargo fmt` reflow had silently defeated the edit
  that removed *"you may read the files named"* from the prompt. Every run after
  that was ordering a model with no tools to go and read files. Two lessons, both
  cheap: an edit that "did not apply" fails silently and looks exactly like a
  behaviour bug, and dumping the real prompt found in one minute what three
  rounds of plausible reasoning had not. There is now a test asserting the prompt
  never offers a tool the pass does not have.
- **No cross-lane severity pass.** A multi-lane report warns that severities are
  not comparable across lanes, but does not yet rank within each lane separately.
- ~~Same model, same input, different answers — unquantified.~~ **Measured, and
  the interesting part is not the count.** Three identical haiku sweeps of the
  fixture returned 5 findings every time, four of them at the same line in all
  three runs and the fifth within one line. So *what* gets found is stable.

  *How it is described* is not. Those three sweeps characterised the same
  `parse_price` weakness as "fails on pence values with fewer than 2 digits",
  "panics on invalid input and accepts ambiguous formats without validation",
  and "does not validate that pence are in the range 0-99" — three different
  defects on the page, one defect in the code.

  That probably explains the agreement result better than anything else here. If
  one model describes one defect three different ways across three runs, two
  *different* models arriving at the same description was never likely. The
  merge threshold was being asked to do a job the inputs cannot support, and
  retiring agreement as a ranking signal looks less like a concession and more
  like the only honest reading.

  (An earlier observation of 5 versus 7 findings stands as a separate data point
  on a different model and configuration; this measurement is haiku on the
  git-backed fixture and does not supersede it.)
- **Clustering was tuned on data that could not stress it.** The seeded fixture's
  findings all land within a line or two of each other, so a 3-line anchor window
  looked fine; on a real crate two vendors anchored the same defect 5 lines apart
  and it failed to merge. Now 10, with the real output committed as a second test
  corpus. Assume other thresholds have the same problem until real data has been
  through them.
- **Compound findings do not merge.** When one model bundles two defects into one
  finding it stays separate from both single-defect counterparts. Documented at
  the threshold in `crates/bugsleuth-judge/src/cluster/pairing.rs`.
- ~~The wording threshold has a thin margin on the wrong-merge side.~~ **It was
  crossed, by a feature added the same afternoon.** Asking one model to sweep the
  same code twice — the new repeat pass — broke the merge rule, and the tool's
  own output is what showed it. Two passes of *one* model word two different
  defects far more alike than two vendors ever did: an underflow in
  `remove_stock`, a division by zero in `average_price` and an index panic in
  `top_by_value`, all inside ten lines, scored 0.36 to 0.42 against each other
  against a 0.35 threshold. All three collapsed into one, and a defect both
  passes had found **disappeared from the report entirely** — the worst failure
  this judge has, because the reader never learns it exists.

  No threshold fixes that; the two populations now overlap. What separates them
  is that they name different code. Merging now also requires the two accounts to
  share a `snake_case` or `camelCase` identifier, which keeps the case the line
  window was widened for — one defect anchored at a signature and at the
  offending comparison, where both accounts still say the same function name.
  Both passes are committed as a fourth test corpus, and both new tests were
  confirmed to fail with the gate removed.

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
