# Progress

**Written for a session with zero context.** If you are resuming, read this file
and `DECISIONS.md`, then continue from "Next concrete step".

Last updated: 2 August 2026.

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

## The self-defect rate, re-measured: still not near zero

Same four lanes, same two vendors, same stripped worktree as the sweep that
found seven criticals this morning. **14 findings, 6 graded critical.** The rate
has not fallen to nothing, and saying otherwise would have been wishful.

What is in those six matters more than the count. Five are one theme: the proof
path runs the reviewed repository's own test suite, so a review that asks for
proof executes untrusted code on the machine. That is not a bug to be patched
away — proving a defect *means* running its test — but it was undocumented, and
worse, the report actively denied it. The first review limit said **"Nothing was
run"**, which is false whenever proving is enabled. The tool built to catch
confidently-wrong claims was making one about itself, in the section whose whole
job is honesty. It now says proving is the exception and names what executes.

Two more were defects in code written the same day:

- **The confirmation dialog had no CSS at all.** A script that appended the
  styles aborted partway on an unrelated error; the button it was also adding
  got fixed by hand, and nobody noticed the rest had never landed. The dialog
  had correct focus, Escape and Tab behaviour — all of which I verified — and no
  overlay whatsoever, so it did not block the app it was guarding. Behaviour was
  tested; appearance was assumed.
- **A failed Stop left the UI stuck**, button dead and status reading
  "Stopping…" forever, with the run still spending behind it.

The honest summary: the rate is falling in severity but not in count, and two of
today's six criticals were introduced today. A tool that finds defects in its
own fresh work is working; a codebase that keeps producing them at this rate is
not yet mature.

## v0.2.0 published on one platform, and why

The first tag failed exactly where the gap it was closing had always been.

`tauri build` produces the app and nothing else. The CLI binary existed in
`target/release` on Windows only because `verify.ps1` builds it as part of the
gate — and `verify.sh`, written the same afternoon, did not. So the release
workflow found the CLI on Windows, published there, and failed on Linux and
macOS with *"expected executable missing: target/release/bugsleuth"*.

Two gates that build different things are two different definitions of green.
That is the third time today the two disagreed: first on how to count lines,
then on which file types to check, now on what to build. Each time the second
platform found it, and each time the fault was in the older, Windows-only side.

Fixed forward rather than by deleting a tag: the release workflow now builds
the CLI explicitly instead of relying on a previous step's leftovers, `verify.sh`
performs the same release build `verify.ps1` does, and **v0.2.1** carries both.
v0.2.0 remains published with Windows-only artifacts and is superseded.

## v0.2.0, and what it is and is not verified by

Cut from `c94822e`, with the gate green on **Windows, Linux and macOS** — the
first release of this project, and the first time any of it ran on a platform
other than the one it was written on.

**Verified before tagging:**

- The gate passes on all three platforms, running the same checks: `verify.ps1`
  on Windows, `verify.sh` elsewhere, counting the same way and checking the
  same file types after the two disagreed and one turned out to be wrong.
- ~200 Rust tests and 33 frontend tests.
- A four-lane sweep of this repository, with every real critical it found fixed.
- The portable Windows binary is genuinely self-contained: its PE import table
  is nothing but Windows system DLLs and UCRT forwarders, with no
  `WebView2Loader.dll` and no vcruntime.
- Recall on known defects is a rate rather than an anecdote: 3 of 3 and 3 of 3
  across repeated runs.

**Not verified, and not claimed:**

- **The live WebDriver acceptance journey has never run.** The app had no
  WebDriver support until today and still does not attach; the remaining work
  is the JavaScript half. The Tauri standard asks for that journey on a
  lifecycle change, and cancellation is one, so this release goes out with that
  evidence missing and said out loud rather than quietly skipped.
- **No binary has been launched on a clean machine.** Import analysis is strong
  evidence that the portable exe needs nothing preinstalled; it is not proof.
- **The Linux and macOS artifacts have never been run at all** — they compile
  and the gate passes there, which is not the same as the app working.

## Repeat runs: the recall result is a rate now, not an anecdote

The fair criticism of every number above was that each condition ran once. A
single run cannot tell a mandate that works from a model that got lucky. So the
two decisive conditions were repeated.

| Condition | Ground truth | Runs | Found |
|---|---|---|---|
| D2, security | `ruleSummary()` interpolated into `innerHTML` | 3 | **3 of 3** |
| D3, contract | frontend size guard above the backend budget | 2 | **2 of 2** |

Both were **missed by both vendors** before the mandates named their class, so
the before/after is not one lucky run either side.

**The per-vendor split is the interesting part, and it is not flattering to the
premise.** On D2 the ground-truth defect was found by Claude in all three runs
and by Codex in none — Codex found the *other*, louder injection in that file
every time and stopped, which is exactly the behaviour the "do not stop at the
first one" instruction was written for, still happening in the vendor it was
written for. On D3 both vendors found it in both runs.

So the mandates hold up across repeats, and one vendor still reliably stops at
the first sink. That is a real limit of the current security lane, measured
rather than suspected.

A repeat run also turned up a defect nobody was looking for — a frontend
filename sanitiser that omits the Windows reserved names — which is a small
argument for repetition beyond measurement.

## The acceptance harness was never able to pass, and why that took eight runs

Worth recording as a method failure, not just a fix.

The suite failed with an empty document. Its own timeout message said *"the page
is blank, the binary is probably a development build — run `cargo clean
--release` then `cargo tauri build`"*. That message was confident, specific, and
wrong, and I followed it: a targeted clean, then a full clean, then a driver
version check, then killing a stale instance, then checking the selector, then
launching the binary by hand. Six runs, all guesses.

The seventh printed `browser.getUrl()` before waiting. **`about:blank`.** One
line, and every earlier hypothesis died at once: the app was fine, the driver
matched the runtime, the selector existed — the webview WebDriver was attached
to simply was not the app's, because **the app had no WebDriver support at
all**. No plugin, no feature, no capability. The harness's client half was
complete and well written; the app half had never existed, so the suite could
not have passed on any day.

The lesson is the one this project keeps relearning: *a diagnostic that asserts
a cause is worse than no diagnostic when the cause is wrong*, because it buys
confident motion in one direction. The spec now prints the URL and the page
source before it waits.

**Fixed on the way**, both real:
- A startup race — wdio connected before `tauri-driver` had bound its port, so a
  driver seconds from ready looked like a missing one.
- `onPrepare` throwing did not stop the run. WebdriverIO logged it and started
  the workers anyway, turning a clear *"the binary does not contain the current
  frontend"* into an unrelated connection error — and, the first time, into 76
  minutes of silence.

**Where it stands.** `tauri-plugin-wdio` is now compiled in behind a non-default
`wdio` feature, its capability is declared inline in a test-only config the
published build never loads, and the release workflow asserts the shipped binary
does not carry it. The driver still attaches to `about:blank`, so the remaining
piece is the JavaScript half — the wdio Tauri service in `wdio.conf.ts`
`services`. **The live acceptance journey has therefore still never run**, and
no release should claim it has.

## Regression sweep before 0.2.0: 33 defects, seven of them real and critical

All four lanes, two vendors, against a frozen worktree with every project
document stripped. 34 findings merged into 33 distinct defects; the triage pass
moved 21 of 33 severities.

Ten graded critical. Two were the seeded fixture's own planted bugs, which is
the answer key being found rather than a defect. Seven were real and are fixed;
the last is Kilo's un-sandboxable execution, which cannot be fixed here and is
stated in every report instead.

Three of the seven are the **same class this codebase keeps producing**:
something destroyed before its replacement is safely written.

- **Settings** were truncated before the replacement landed, so a failed save
  wiped every setting. This module's own error reporting exists because losing
  settings silently once cost a whole session — and it was still possible to
  lose them this way.
- **The fix-prompt bundle** was truncated in place. The earlier fix reordered
  the *per-defect* files and did nothing for the bundle itself.
- Both now stage and rename, like the sweep reports.

Two are **a comment describing a fix that was never made**:

- A sweep whose task panicked printed a warning and vanished from the report,
  reading exactly like a lane that ran and found nothing — beside a comment
  that had demanded a gap for weeks.
- The Kilo repair's own description says it gets no repository access, while
  the code handed it the worktree under review, with `--auto`, so any tool call
  was approved unasked. It now runs in an empty directory with nothing to reach.

And two are keys that were not keys:

- **The run cache** was keyed by the repository's leaf folder name, so a
  worktree beside the original shared a directory and resume handed one repo
  the other's sweeps while the report stated the wrong provenance.
- **Turn exhaustion** was only recognised on a non-zero exit, so the identical
  failure with a clean exit bypassed the salvage entirely.

The seventh is the one worth dwelling on. **The test-run logs were written
inside the tree being reviewed** — `<repo>/target/bugsleuth/`. A repository can
commit a symlink at that path, git materialises it on checkout, and
`File::create` follows it and truncates whatever it points at. Arbitrary file
destruction, chosen by the code under review. It is the same escape the anchor
check was hardened against a day earlier, in the one place that writes rather
than reads — which is exactly the lesson the security mandate now carries:
having found one instance of a sink, go and find the others.

## The last two lanes reviewed BugSleuth, and the salvage saved the haul

Correctness and UX had never been pointed at this repository. Seventeen findings
between them; these are the ones confirmed by reading the code and fixed.

**The whole correctness haul was salvaged.** Claude's sweep ran out of turns and
was recovered — all ten of its findings, including the two below, would have
been lost entirely under the build from that morning.

Correctness found, and both vendors independently found the first:

- **`write_all` deleted the previous run's work orders before writing anything.**
  A bundle write that failed left the old files gone and the new ones never
  created — tens of minutes of paid sweeping destroyed by a failure that had
  nothing to do with it. This is the *destroy-before-commit* class the mandate
  was taught to hunt the day before, found in code written the same week.
- **`write_report` truncated in place**, so a process killed mid-write left half
  a report. Resume treats an unparseable report as absent, so the cost was
  paying twice for one sweep and losing the first result.
- **Agreement was counted against the number of sweeps, not models.** Every
  report produced that day printed "found by 1 of 3 models" when two models had
  run. The denominator stops being the model count the moment one model covers
  two lanes or repeats a pass.

UX found, against the app's own window:

- **The Quit button's tooltip described what closing the window does** — "Close
  the window to keep BugSleuth running in the tray" — on the button that exits
  immediately. A user reading it mid-run would believe they were backgrounding
  the app.
- **Quitting during a run threw away the review without asking**, and a preset
  button replaced the entire matrix with no confirmation and no undo, sitting
  one click away from "Add model".
- **The status bar had no `aria-live`.** That bar is the app's only channel for
  "settings are not being saved" — the silent failure a comment three lines
  above describes as a real prior incident — and a screen-reader user was never
  told.

Both confirmations only fire when something would actually be lost. Switching
between untouched presets asks nothing, because a confirmation that appears when
there is nothing at stake teaches people to click through without reading.

**Left undone deliberately:** there is still no way to cancel a run once
started. That is a real gap and a real feature, not a one-line guard, so it is
recorded here rather than half-built.

## The second self-review found three more, and the salvage paid for itself

Run again after the mandate work, on a frozen worktree of HEAD with every
project document stripped. Three findings, none overlapping the three fixed
after the first pass, all three confirmed real by reading the code:

- **Kilo sweeps run with the user's own permissions** — and the throwaway
  worktree, which is easy to read as containment, only stops Kilo modifying the
  code under review. Since the reviewed repository is untrusted by design, text
  inside it can address the agent directly. This is not fixable in BugSleuth:
  Kilo has no per-invocation permission flag the way Claude has a tool allowlist
  and Codex has `--sandbox read-only`. So it is now *stated* — before the run
  while the choice is still free, and in both report renderers afterwards.
- **The proof-count cap was advertised and never enforced.** `max="25"` on a
  number input only marks the field invalid when a value is typed; it neither
  clamps nor blocks reading. Typing 500 asked for 500 proof attempts, each a
  model invocation and a full test run.
- **The proof count was never made an integer**, so `1.5` reached Tauri as a
  JSON float, failed to deserialize into `usize`, and stopped settings saving
  *and* runs starting — with a raw deserialization error.

The last two are the same defect class the contract mandate had just been taught
to look for: one rule written on both sides of a boundary with nothing making
the two agree. A test now reads `index.html` and asserts the advertised cap and
the enforced cap are the same number, so they cannot drift apart again.

**The salvage feature paid for itself on its first real run.** Two of the four
sweeps hit `error_max_turns` and were recovered; both are marked RECOVERED in
the report. One of them is where Claude's contract finding came from — under the
previous build that whole sweep, and that finding, would have been lost.

## Security and contract, measured at last: 3 of 3 after naming two classes

Correctness had a recall number and UX had a precision number. These two lanes
had neither, and running them on BugSleuth itself did not fix that — I graded
those three findings myself, which is the one thing this project's evidence says
not to do.

**Precision, graded by someone else.** All three self-review findings went to an
independent strict grader *and* to an adversarial skeptic told to refute by
default. **3 of 3 survived both.** One skeptic did better than my own check: it
copied the two accused functions verbatim into a standalone program and ran them
against a real directory junction, confirming the symlink escape empirically
rather than by reading. One correction it forced: the Codex scratch-directory
finding was graded **too severe** at high.

**Recall, against ground truth chosen independently of the tool.** Seven fix
commits in Alder's history were verified as genuine security or contract
defects — chosen by what they fixed, never by what BugSleuth had found, each one
confirmed present at its parent commit. None were rejected. Four were findable
from code alone; the other three needed knowledge of a remote API, which is
itself the evidence behind one of the review limits now printed in every report.

Three were measured, each swept at its own parent commit with every project
document stripped from the tree:

| Known defect | Lane | Before | After |
|---|---|---|---|
| Draft body written to `innerHTML` unsanitised | Security | Claude only | Claude only |
| `ruleSummary()` interpolated into `innerHTML` | Security | **Missed by both** | **Found** |
| Frontend size guard larger than the backend budget | Contract | **Missed by both** | **Found by both** |

**1 of 3, then 3 of 3** — the same trajectory the correctness lane took, by the
same route, and the misses were nameable both times.

The security miss was *stopping at the loudest instance*. Both vendors found one
unescaped `innerHTML` assignment in the file, reported it, and never looked at
the other one. The mandate now says to enumerate every other use of a sink once
one is found, to follow helpers that build strings assigned to a sink, and to
report each unsafe use separately. The effect was visible immediately: Codex
went from one finding to two distinct sinks in the same file.

The contract miss was *a rule written down twice* — a frontend attachment guard
of 3 MiB against a smaller backend budget, both plain constants in the
repository. The mandate now asks for limits duplicated across a boundary and for
both values to be named. Both vendors found it on the re-run.

**What this does not establish.** Three defects, one run per condition, one
repository. It does not give either lane a recall *rate*, and model variance
contributes something. What it does establish is that both lanes were missing
findable defects for a nameable reason, and that naming the reason moved them —
which is now the third time that has worked.

## BugSleuth reviewed itself, and found three real defects

The security and contract lanes had never been measured — correctness and UX
had. So BugSleuth was pointed at its own code: a frozen worktree of HEAD with
every project document removed first, so findings had to be discovered rather
than read off `PROGRESS.md`. Two vendors, two lanes.

Three findings, three real, one graded critical. All three are now fixed, each
with a test that fails without the fix:

1. **Anchor verification followed symlinks.** The path check ran on the path's
   *components* — no `..`, no drive letter — which is a lexical test that says
   nothing about where a path resolves. A reviewed repository containing a
   symlink at an innocent name like `src/util.rs` could point it at a private
   key or at the user's own settings, and BugSleuth would read that file, quote
   it into the report, and hand it to whatever agent got the fix prompt. The
   reviewed repository is untrusted input by design, so this was the real thing.
   Containment is now checked against the resolved path.

   Proven, not assumed: a test creates a directory link out of the repository
   and asserts the quote is refused. With the guard removed it fails with "a
   link out of the repository was followed and its contents quoted". (Windows
   needs elevation for *file* symlinks, so that variant skips and says so; the
   directory-junction variant runs on an ordinary machine.)

2. **The Codex working directory was predictable and reusable.**
   `bugsleuth-codex-<pid>` in the shared temp area, created with
   `create_dir_all`, which succeeds on a directory that already exists. Anything
   already sitting there was adopted: its `answer.json` would have been read
   back as a review — a forged finding list — and the directory deleted
   afterwards. Now created exclusively, so the directory is ours because
   creating it is what proved it did not exist.

3. **Report filenames were lossy.** Every non-alphanumeric character became a
   dash, so `codex:a/b` and `codex:a-b` shared one file: one sweep overwrote the
   other and a resumed run handed a model another model's findings while the
   report stated the wrong provenance. The encoding is now injective. Reports
   written under the old scheme are still reused — they cost tens of minutes
   each — but only after the new name is tried, and only when the report inside
   says it is that sweep.

**What this says about the tool.** Two lanes that had never been measured found
three real defects in a codebase already reviewed by hand many times this week,
and one of them was a genuine security hole in the part of BugSleuth whose whole
job is to be untrusting. Small sample, but it is the first evidence that the
security and contract mandates do anything at all.

One sweep of four failed (`error_max_turns` on Claude's contract lane) and the
report says so rather than reading as three clean lanes.

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

## Attacking the arrival rate, not the detection rate

**The concern this answers:** the tool keeps finding real defects in its own
fresh work, and a codebase producing them at that rate is a capable internal
tool rather than a finished product. Finding them faster does not fix that.
Making whole classes of them impossible to commit does.

Five of the classes BugSleuth has repeatedly found in itself now fail the build
mechanically. Each was mutation-tested — the defect was reintroduced on purpose
and the build was watched to fail — because a check nobody has seen fail is a
check nobody should believe.

| Class | What now catches it | Proven by |
|---|---|---|
| A call into Rust whose failure is ignored | Every `invoke` must handle rejection or carry a written reason | The Stop-button defect, re-created |
| A class name with no style rule | Markup, code and stylesheet compared both ways | Deleting the dialog CSS again |
| An element id the markup no longer has | Every lookup checked against `index.html` | Renaming an id and a whole prefix |
| A name that crosses between the window and Rust | Commands, events, settings fields and lane names all compared | Renaming each, one at a time |
| Destroying a file before its replacement is safe | One atomic write, and a test that nothing bypasses it | Blocking the staging path mid-write |

**Two of them found live defects the moment they were written.** `plan_run` was
exposed over the app's command channel with no caller anywhere — surface with no
user. And the command line truncated your `--json-out` and `--patch-out` in
place, so a failure partway through destroyed the previous report: the same
destroy-before-commit defect the tool had already found in three other places
and had fixed three separate times, each with its own copy of the same six
lines. There is one copy now.

**Two of the new checks were themselves wrong first time, and that is the
uncomfortable part.** The rejection rule reported a correctly-handled call as
unhandled because it stopped reading at the first semicolon. The lane-name check
matched the wrong block, produced an empty list, and *passed* while a lane was
renamed — an empty set satisfies every assertion downstream of it. Both were
caught by testing the checks rather than trusting them, and every derived list
in that code now has to prove it is non-empty before anything is concluded from
it. The lesson generalises: the failure mode of a check is silence, not noise.

**The rate was then measured, and it has not fallen.** A fresh self-review
found 17 findings, 4 critical, against 14 findings and 6 critical last time. All
14 that were verified survived an adversarial pass told to refute them. Dating
each defect to the line that is actually wrong: **6 came from day one, 2 from
day two, 5 from today**. Per thousand lines written that is roughly *double* day
one's rate. The newest code is not safer.

## What actually causes them, and the three controls now in place

Eleven defects arrived here in one day. Classified by cause rather than by file,
they are not eleven mistakes. They are two.

**Six of eleven: a check matched less than existed and returned a smaller answer
instead of an error.** A statement that ended at the first semicolon, above the
handler it was looking for. A pattern anchored on a bare name that found an
import instead of a declaration, produced an empty list — and an empty list
satisfies every comparison, so the thing it was guarding was renamed and the
test passed. One pattern per code layout, reading two cases of three. A file
split that dropped a whole block of tests nobody noticed existed.

**Three of eleven: something was computed and nothing consumed it.** The gate's
exit code, swallowed by a pipe, so a commit the gate had rejected went to the
remote anyway. A failure with no handler. Values produced and never read.

Three controls, each verified by causing the failure on purpose:

1. **A parser instead of regular expressions.** TypeScript was already a
   dependency — it is what the build type-checks with — so its own parser was
   already here. It cannot stop early, cannot match the wrong block, cannot read
   a comment as code. Every syntax question the checks ask now goes through it,
   and the helpers are tested against snippets whose right answer is written out
   by hand. *Verified: restoring the folder-picker defect verbatim fails the
   build; so does dropping a field from an event, or adding one to settings.*

2. **A pre-push hook that runs the gate.** A person remembering to read an exit
   code is not a control. *Verified: a deliberately over-cap file is refused at
   push.*

3. **`tests.lock`, the name of every test, checked by the gate.** Splitting files
   under the line cap lost tests twice in one day, and the count did not move
   either time because tests were added in the same change. Names are the only
   thing a comparison can be trusted on. *Verified: deleting one test names it
   and fails the build.*

The script written to generate that inventory silently wrote only the Rust half
on its first run — one pattern matched nothing — which is the same failure, in
the file written to catch it. It now refuses to write an inventory where either
half is implausibly small, and says to fix the scan rather than the threshold.

**What this still does not claim.** These are controls on how defects are
*caught*, and two of the three are about the checks rather than the product. No
re-measurement has happened since they landed. The honest statement is that the
two dominant causes now have mechanical answers, and whether that moves the rate
is a question only the next self-review can settle.

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

## All ten confirmed defects fixed, each with a test that fails without it

The 2 August self-review produced 17 findings. Fourteen were verified by an
adversarial pass told to refute them; all fourteen survived, though four had
their severity argued down. Four were fixed the same evening; the remaining ten
are now done.

| What was wrong | What a user was losing |
|---|---|
| A reply with no `findings` key parsed as an empty list | Real findings gone, reported as a clean sweep, no warning |
| Resume matched on lane and model, never scope | A report claiming to have reviewed code it never opened |
| A fixed three-backtick fence around quoted code | Reviewed code could address the fix-prompt's agent directly |
| Clustering never compared lanes | A security defect merged into a correctness one and vanished |
| "Cut short" flag dropped in two of three reports | A partial list of findings read as the complete one |
| Cancellation discarded finished sweeps | Minutes of paid work thrown away on a coin toss |
| Cancellation compared a bare alias to a resolved label | Lanes already swept reported as not reached |
| Proving never said it runs your code unsandboxed | A security exposure the report left the reader to infer |
| The proof cap existed only in the window | A hand-edited settings file could ask for any number |
| The confirmation run ignored its own outcome | A timeout reported as "no test matches that name" |
| Lane toggles dropped keyboard focus to the top of the page | A keyboard user re-tabbing in on every single tick |
| The progress log replaced its whole text each event | A screen reader re-reading the entire log to announce one line |
| Two runs shared a scratch directory | One run deleting the other's work mid-test |

The two interface fixes were verified in a real browser rather than by reading:
focus stays on the same control across a rebuild and falls to the top of the
page without the fix, and the log appends without running lines together.

## Shipping, not just building

- **Install instructions that are true.** The release publishes fourteen
  artifacts and the README opened by telling you to build from source. It now
  leads with the single file that runs with nothing installed, and a check
  compares the download table against the release workflow — renaming an asset
  or dropping a platform fails the build. A download table is the first thing a
  new user acts on and prose does not fail to compile.
- **v0.2.2 published** on Windows, Linux and macOS: portable binaries,
  installers, CLI binaries and checksums, all three CI jobs green.

## The rate, measured a second time: 17 findings, then 9

Same conditions both times — four lanes, the same two vendors, a stripped
worktree. **17 findings and 4 critical, then 9 findings and 4 critical.** The
count roughly halved. The critical count did not move.

That second number needs reading carefully, and the honest reading is mixed.

**Two of the four criticals were already handled.** The Kilo repair pass was
flagged for executing tools from attacker-controlled output; it already points
at an empty private directory for exactly that reason, and says so in a comment
written the day before. The unsandboxed-vendor caution already covers the rest.
So the tool re-reported a documented, accepted trade-off as a critical defect —
which is the right conservative behaviour for a reviewer, and also means "4
critical" overstates what was actually wrong.

**Three of the nine were in code written that same day**, one of them hours
earlier:

- The atomic write staged into a fixed filename, so two writers of one report
  shared a temporary file and the survivor could hold the other's bytes. The
  worktree module had learned that exact lesson an hour before and this one was
  written without it.
- The cancellation bookkeeping struck off every pass of a model when one
  finished, so a run cancelled after pass one of three claimed the other two
  were accounted for. That was introduced by the fix to the defect beside it.
- Confirming Stop after a run had already finished overwrote "Finished" with a
  permanent, false "Stopping…".

**The most interesting one was a lie in a doc comment.** Clustering claimed
order-independence in its own documentation and did not have it: a finding
joined the *first* cluster it matched, and the matching relation is symmetric
but not transitive. A compound finding bridging two defects merged them in one
order and left them apart in another, so which report a user saw depended on the
order sweeps happened to finish in. The comment asserting the opposite is
probably why it survived being read. It is the connected components now, and the
old algorithm fails the new test concretely.

**What this says about the arrival rate.** Halving the count is real and the
guards deserve some of the credit — several of the classes closed simply did not
reappear. But new code still produced defects at a similar rate to old code, and
one of them was created by a fix to another. The mandates catch more than they
used to; the writing has not got proportionally safer. That is the honest
summary, and it is why the count is reported rather than a conclusion.

## Next concrete step

**Ten confirmed defects remain from the 2 August self-review**, verified and
ordered by what a user actually loses. Worst first:

1. **Proving runs the reviewed repository's own build and test code with your
   full permissions and no sandbox** — three times per defect. A malicious or
   merely broken build script can read anything your account can. Not a sandbox
   build-out; the fix is to say so, next to the warning this codebase already
   carries for the same risk from Kilo, and to record the trade-off.
2. **Cancelling a run can throw away a sweep that already finished** and was
   already paid for.
3. **The "this sweep was cut short" flag is dropped in two separate report
   paths**, so a truncated lane reads as a complete one. The same mistake made
   twice, worth one pass.
4. **Toggling a lane checkbox drops keyboard focus to the top of the page**, on
   the app's busiest control.
5. **A screen reader re-reads the whole progress log** on every update.
6. Two concurrent runs against one repository delete each other's scratch
   worktrees; a confirmatory test rerun reports the wrong reason on a timeout;
   a cancelled run can contradict itself in the report; the backend does not
   enforce the proof cap the window does.

**Then re-measure.** The count that matters is defects in code written *after*
the three controls landed. Same conditions or the numbers are not comparable:
four lanes, the same two vendors, a stripped worktree.

```bash
cargo run -p bugsleuth-cli -- run --repo . --config bugsleuth.example.json --out-dir runs/ --resume
```

Two things to hold to when reading the result:

1. **A lower count is not the same as a lower rate.** Much of the code reviewed
   last time has not changed, so its defects will simply be found again.
2. **Check what the new controls did not catch.** Every finding outside the
   known classes is the answer to "what should be closed next", and it is a
   better answer than any guess made in advance.

## Precision on a second codebase: 14 of 14

Every precision and recall number this project had came from one repository. A
reviewer measured only where its mandates were written is measuring how well it
was tuned, not how well it works, so the number that mattered next was one from
code the mandates had never seen.

**Setup.** OnTop, 7 Rust files and roughly 3,500 lines, reviewed by three
vendors across all four lanes: Claude on correctness, security, contract and UX;
Codex on correctness and security; Kilo on correctness. Kilo's sweep was lost to
a context-overflow error in its own CLI and is recorded as NOT SWEPT rather than
quietly dropped. Sixteen findings from the six sweeps that ran merged into
fourteen distinct defects: three critical, six high, five medium.

**Grading.** Every one of the fourteen went to three independent sceptics, told
to assume the finding was wrong, to answer "not real" when unsure, and that a
style preference or a hypothetical with no concrete trigger does not count. The
same standard that produced 78% on the first corpus, so the numbers compare.

**14 of 14 are real, and all three judges agreed on every one.** No finding
split its panel.

The graders did the work rather than deferring to the report: the median
verdict cites 456 characters of what was actually read or searched, and none
cited less than sixty. One went into the pinned egui 0.31.1 source to confirm
`widget_info` is never called before accepting an accessibility finding.

### What the number does not say

**Zero rejections means zero information about how the reviewer fails.** A clean
sweep cannot show that a failure mode is gone, only that it did not occur in
fourteen findings. The failure-shape analysis returned nothing to learn from,
which is the correct answer to an empty set and not a result.

**Precision says nothing about recall.** 14 of 14 is equally consistent with a
sweep that found everything and one that found the easy third. Nothing here
measures what OnTop contains that the review walked past, and that is now the
axis with no data on it at all.

**One caveat that cannot be resolved from here.** The sceptics graded from the
fix prompts, which carry code quotes already verified to exist at the cited
line. That anchor check is a real precision mechanism and plausibly explains
genuine improvement — but whether the 78% corpus was graded from equally
supported material is not recorded, so the two are not certainly like for like.

The claim this supports is "comfortably above 80% on a codebase it was not tuned
for". 100% is what this sample produced, not a rate to expect.
