# Night report

Unattended session, 31 July – 1 August 2026. Written for someone who does not
read Rust.

**Two headlines.**

**One: the experiment that decides the project succeeded, and I have the
evidence to back it.** A model asked to prove a defect with a failing test did
so on both real defects it was given, and correctly refused on both fabricated
ones. Section 2 — that is the part worth your attention.

**Two: the cross-vendor premise holds, on weak evidence.** All three vendors —
Claude, Codex and Kilo — work. On the same code with the same instructions
they produced 23 findings that merge into 11 distinct defects, and **every
vendor missed something another one found**. Running only the best single vendor
would have cost you 2 of the 11. Sections 4 and 5.

I would not call that premise proven, and section 5 explains why I think the
test was too easy.

**Three: the number that should worry you.** On a *real* repository, with the
lane pointed straight at the file, **two vendors both failed to find the bug the
experiment was built around.** The toy fixture flatters this tool badly. Section
5a.

Nothing here is production-ready.

---

## 1. What got built

Five small Rust libraries and a command-line harness. No user interface — that
was out of scope, and remains where your input is worth most.

| Piece | What it does |
|---|---|
| `domain` | The vocabulary: lanes, findings, proof verdicts. Pure definitions, no behaviour |
| `provider` | Drives the Claude Code, Codex and Kilo CLIs as subprocesses, using your signed-in sessions |
| `judge` | Merges findings from several vendors into one ranked list of distinct defects |
| `verify` | Checks findings against reality: does this code exist? Does this test really fail? |
| `bugsleuth` | The command-line harness that ties them together |

Five commands work today. The one that matters is `run`, which does the whole
job in a single step: sweep every configured model against every lane it covers,
merge the results into one ranked list of distinct defects, and optionally prove
the top few with failing tests.

```bash
bugsleuth preflight
```

```bash
bugsleuth sweep --repo <path> --lane correctness --model sonnet
```

```bash
bugsleuth sweep --repo <path> --lane correctness --model codex:
```

```bash
bugsleuth sweep --repo <path> --lane correctness --model kilo:
```

```bash
bugsleuth prove --repo <path> --defect-file <file> --test-command "cargo test"
```

```bash
bugsleuth run --repo <path> --config bugsleuth.example.json --out-dir runs/ --resume
```

```bash
bugsleuth run --repo <path> --config c.json --prove-top 5 --test-command "cargo test"
```

```bash
bugsleuth judge run-a.json run-b.json run-c.json
```

## 2. The experiment that decides the project

Your brief said this was the highest-value work available, and that a
well-evidenced negative result would be worth more than working code. It is not
a negative result.

### What was being tested

BugSleuth's whole premise is that a finding should arrive with a **failing test**
that demonstrates it, because you cannot check a finding by reading the code. To
you, a confident hallucination and a real bug look the same. A test that fails
is something a machine can check on your behalf.

That premise rests on two assumptions, and both had to be tested:

**(a)** Can a model actually write a test that fails because of a defect it
claims? If not, there is no proof mechanism.

**(b)** Does requiring a failing test actually filter out false positives — or
will a model happily manufacture a "failing test" for a bug that does not exist?
If it will, the filter gives false confidence, which is worse than no filter.

### How it was set up

I used **Alder**, your own repository, as the realistic case. I cloned it
into a scratch directory — your actual repository was never touched — and rolled
it back to just before commit `REDACTED`, which fixed a real bug: the "load
images" banner missed remote images written with non-standard spacing, so images
were silently stripped with no way to load them.

That rollback point matters. **All 50 tests pass there.** Running the test suite
teaches a reviewer nothing. That is precisely the situation BugSleuth exists for.

I also used a small purpose-built repository with five deliberate bugs.

The model was given a description of a defect and asked to prove it. It was
never told whether the defect was real.

### The four runs

| # | Repository | Defect | Result | Correct? |
|---|---|---|---|---|
| 1 | Alder (real code) | The real banner bug | **PROVED** | Yes |
| 2 | Alder (real code) | Fabricated: a CSS filter can be bypassed with escape sequences | **Refused** | Yes |
| 3 | Fixture | Real: stock removal underflows | **PROVED** | Yes |
| 4 | Fixture | Fabricated: a missing-item check is absent | **Refused** | Yes |

**Four for four.**

### What "PROVED" actually means

For run 1, the harness observed all of this by running the tests itself:

- Before the attempt: **50 tests passed, 0 failed.**
- After the attempt: **50 tests passed, 1 failed** — the model's new test.
- The model changed only the test section of one file. No production code.
- Then, separately, I applied the real historical bug fix and re-ran the model's
  test: **51 passed, 0 failed.**

That last step is the one that makes it proof rather than a red mark. A test that
fails on the broken code *and* passes on the fixed code is genuinely detecting
the bug, not failing for some unrelated reason.

The model's test turned out to be essentially the same test the real bug-fix
commit added, arrived at independently.

### What the refusals looked like

This was the more surprising result. On run 2 the model came back with, in
substance: the function does not work the way the description claims — it rejects
backslashes outright, which closes off the entire attack class described, and
there is already a test proving it. It went further and wrote a temporary test to
confirm the specific payload was blocked, then deleted it, leaving the repository
clean.

Run 4 was the same shape: it quoted the actual line of code, pointed out the
check the description claimed was missing is right there, and named the existing
test that covers it.

Neither run produced a fake proof. Both explained themselves accurately.

### What this does and does not establish

**Established, by observation:** on four cases, the mechanism worked. Real
defects were demonstrated with genuine failing tests; fabricated ones were
refused with correct reasoning.

**Not established.** Four cases is four cases. Both real defects were reasonably
easy to test — small, pure functions with no I/O. I would expect a much lower
success rate on defects involving concurrency, timing, or a network. Both
fabricated defects were *clearly* false; a subtly wrong claim would be a harder
test and I did not run one.

One methodological note in the interests of honesty. My first draft of the
fabricated Alder defect claimed the code missed protocol-relative image URLs.
I checked before running it and found that claim was **true** — so it would have
measured the exact opposite of what it was meant to. I replaced it. Had I not
checked, the experiment would have produced a confident wrong conclusion. That
is worth knowing about the eval, not just about the tool.

### The check that matters most, and why

A coding agent asked to make a test fail can always succeed by breaking the code.
That produces a red test that says nothing about the original defect — a false
proof, and the most dangerous possible output.

So the harness counts how many tests passed before the attempt and after. If the
number went **down**, production code was changed, and the attempt is rejected
outright no matter what the model claims. The model is also told this in advance.

This never triggered in the four runs. Because it is the load-bearing check, I
covered it with direct tests rather than leaving it unexercised.

## 3. M0: does the review itself work?

Before the proof work, the basic sweep was verified against the fixture
containing five deliberate bugs.

**It found all five,** in eight turns, and every one was anchored to real code.
None was discarded.

The fifth was the interesting one: the code applies a discount at "more than 10
items" while its own documentation says "10 or more". Finding it requires
comparing intent against implementation, not just reading code.

One caveat: that fixture is small and its bugs are blatant. It proves the
plumbing works. It does not tell you what the detection rate is on a real
codebase, which I did not measure.

## 4. Cross-vendor: the premise, tested

BugSleuth exists because asking one model to review another model's work has
correlated blind spots. That is a claim, and until tonight it was untested here.

I ran the **same lane, on the same code, with the same instructions**, through
both vendors.

| Vendor | Findings | Discarded |
|---|---|---|
| Claude (sonnet) | 5 | 0 |
| Codex (default model) | **9** | 0 |

(A later identical Claude run returned 7 rather than 5 — see the note on
run-to-run variance below.)

Codex found all five that Claude found, and four more:

- Three overflow paths Claude did not mention at all.
- A discount function that underflows if given a percentage above 100.
- **A real bug that was in the fixture by accident.** `parse_price("12.3")`
  returns 1203 pence when it should return 1230 - it treats the part after the
  decimal point as a count of pence rather than as decimal digits.

That last one is the interesting one. I wrote that fixture and its answer key
myself, and I did not notice it. Claude did not report it. Codex did. I then ran
the function to confirm: it returns 1203.

I have left the bug in the fixture and added it to the answer key, noting that
the tool found it rather than me. A fixture curated to match what the tool
already finds is not much of a test.

**What this establishes:** on one lane, on one small repository, two vendors
produced materially different results, and the difference was real signal rather
than noise - every extra finding was a genuine defect, and all nine anchored to
real code.

**What it does not:** one comparison on one small repository. It does not tell
you Codex is better than Claude in general. The honest reading is narrower and
more useful: **they differ, and the difference contained real defects that the
other missed.** That is precisely the argument for running both, which is the
argument the tool is built on.

## 5. M2 and the kill gate

Your brief set a stopping condition: **if the judge's merged top-10 is no better
than the best single model's top-10, the premise is wrong and I should stop
building.** All three vendors now work, so I could run it.

### The measurement

Same lane, same repository, three vendors. 23 raw findings merged into **11
distinct defects**.

| If you ran only... | Defects you would get | Defects you would miss |
|---|---|---|
| Claude | 7 | 4 |
| Codex | 9 | 2 |
| Kilo | 6 | 5 |
| **All three, merged** | **11** | — |

**Every vendor misses something another one finds.** The merged list strictly
beats the best single vendor, so the gate is passed rather than failed.

The merge also produces something no single model can: an agreement count. The
three highest-ranked defects were each found independently by all three vendors.
For a reader who cannot check the code, "all three found this separately" is a
much stronger signal than any one model's confidence.

### Why I do not think this settles it

I want to be careful here, because this is the result most likely to be
over-read.

The fixture is small and its bugs are blatant. All three vendors found nearly
all of them, so the *differences* between vendors are concentrated in the
marginal extras — mostly integer-overflow paths that need extreme inputs to
reach. Those are real defects, but they are not the kind that would change your
decision to ship.

So the honest verdict is: **the gate is passed on the evidence available, and
the evidence available is weak.** The premise survives; it has not been
confirmed. Settling it properly needs a real repository where recall is well
below 100% and the vendors actually disagree about the important things. That is
the next experiment, and it is cheap now that all three adapters work.

I have not stopped building, because the gate did not fail. But I would not
present the multi-model premise as proven on this basis.

## 5a. Recall on real code: the result that matters most

Everything above is measured on a fixture I wrote, whose bugs are blatant. This
is what happened on real code.

**Setup.** Alder, cloned to a scratch directory and rolled back to just before
commit `REDACTED` — the point where the "load images" banner bug exists and all
50 tests pass. The correctness lane, scoped to `crates/infrastructure/src`, the
crate the bug lives in.

| Vendor | Findings | Found the reverted bug? |
|---|---|---|
| Claude | 2 | **No** |
| Codex | 2 | **No** |
| Kilo | failed to run | — |

**Neither vendor found it.** Not because they returned nothing — all four
findings are real, anchored, and worth having:

- an attachment filter that selects one field and filters on another, so real
  file attachments are silently dropped;
- a refresh-token write that can corrupt the previously stored token;
- a timezone heuristic that puts all-day events on the wrong day in UTC+12 and
  above — **found independently by both vendors**, which is exactly the
  agreement signal the whole design is built to surface.

But the defect the experiment was designed around was missed by both, on a
27-file crate, with the search pointed at it.

**What I think this means.** On the fixture, recall looks like 100%. On real
code, for this one defect, it is 0 of 2. One defect is far too small a sample to
conclude the tool has poor recall — but it is more than enough to conclude that
**the fixture cannot tell you what recall is**, and that every encouraging number
in sections 3 to 5 should be read in that light.

**Part of it is a design gap, not just a miss.** The banner bug is a *silent UI
failure* living in backend Rust. The UX lane's mandate covers exactly that
("a failure that is swallowed so the user sees nothing") but its file filter
rejects `.rs` outright; the correctness lane can see the file but its mandate
does not point at user-visible consequences. So the defect currently falls
between lanes. I have flagged this rather than patched it — widening the UX lane
to all files risks precisely the aesthetic-opinion pollution the filter exists to
prevent, and that is your call.

## 6. Bugs found in my own work

Worth recording, because they are the kind you could not catch by reading a diff.
The last three were found only by *running* the thing, not by reading it.

1. **The anchor matcher picked the wrong occurrence.** When a snippet appeared
   more than once in a file, it took the first rather than the one nearest where
   the model said it was. Caught by a test.
2. **A match could start on a blank line**, dragging the reported line number
   backwards. Caught by a test.
3. **My own tests raced each other** — several shared one temporary directory and
   overwrote it while others were reading. This produced a failure that looked
   like a logic bug and was not.
4. **A completed run could lose its output** because the harness did not create
   the output directory. That wasted real quota after the work was already paid
   for.
5. **Test output could have hung the harness.** Fixed by writing it to files
   rather than pipes, and killing a suite that outstays its timeout — a model can
   write an infinite loop into a test.
6. **The judge silently reported the same defect twice.** Two vendors describing
   one bug as "remove_stock underflows" and "Removing more stock ... underflows"
   were not merged, because the wording comparison treated `average_price` as one
   opaque word and could not see past grammar. Found by checking the merge
   against hand-labelled real output rather than by trusting it.
7. **The Kilo adapter glued a reply to itself.** Kilo emits each message's
   complete text more than once, and those are not incremental pieces.
   Concatenating them produced two valid JSON objects run together into one
   invalid document. The first real Kilo sweep failed on exactly this.
8. **Throwaway worktrees survived their own cleanup**, leaving the reviewed
   repository dirty — the one thing BugSleuth promises not to do. Once cargo has
   built inside a worktree, its paths exceed the Windows limit and `git worktree
   remove` gives up. Only visible by checking the repository afterwards.
9. **The packaged app shipped a blank window.** `ui/dist` contained the
   stylesheet but not the JavaScript, while `index.html` referenced it. Every
   check downstream passed — vite build, the Tauri build, the installer, and the
   app itself, which launched and showed a window. The window was simply empty.
   Nothing that looks at exit codes or at "did a window appear" could see it;
   only the WebDriver run, by finding no elements on the page. There is now a
   check that parses the built index.html and confirms every asset it names is
   present.
10. **The judge reported one defect as two, with no agreement.** Two vendors found
   the same timezone bug but anchored it 5 lines apart; my clustering window was
   3. The seeded fixture could never have caught this — all its findings land
   within a line of each other — so the threshold looked fine until real output
   went through it. This is the clearest argument in the whole report for testing
   against real data: the *first* threshold to meet real code was wrong.

## 7. Notable things learned from Eir

Full detail is in `DECISIONS.md`. Three that changed what I built:

- **Eir does not use Tauri's shell plugin or sidecars at all.** Its Tauri app
  never launches a CLI. This means BugSleuth needs none of that machinery either
  — simpler and tighter than the brief anticipated.
- **Eir has no check that a CLI is installed and signed in.** It finds out when a
  call fails. Fine for one call; poor for a long queue. BugSleuth has a preflight
  command. It is not a complete check — being installed does not prove being
  signed in — and the code says so.
- **A comment in Eir records a production deadlock**: writing a long prompt to a
  CLI before reading its output hangs once the prompt exceeds the operating
  system's pipe buffer. BugSleuth does it the correct way from the start, with
  the reasoning preserved. That is a bug I would otherwise have shipped.

## 8. Quota

**Used: 14.** No rate limit was hit, but one Kilo sweep died with the silent
non-zero exit these CLIs use for an overload when three ran concurrently. That
is why sweeps are now batched one-per-vendor rather than fired off together.

## 9. Built after the experiments

Everything above is measurement. These are the parts built once the premise had
survived, all covered by tests but **not yet exercised against real models**
end to end — the individual pieces have been, the assembled `run` has not:

- **A judge**, merging findings from several vendors into one ranked list with an
  agreement count. Deterministic, not another model call — see `DECISIONS.md`
  2.16 for why the cheap version had to be beaten first.
- **`run`**, which enumerates the whole (model x lane) product. Sweeps are
  batched so no two invocations of the same vendor run at once, which is the
  quota governor in its simplest useful form.
- **`--resume`**, because a sweep costs real quota and takes tens of minutes, so
  a run that died at unit nine of twelve must not start over.
- **`--prove-top N`**, which closes the loop: the top N merged defects get a
  proof attempt automatically instead of proof being a separate manual step.
- **The third vendor, Kilo**, at your request. It needed different handling —
  see section 6.

Three traps I built and then removed, all of the same shape: nothing crashes,
the output just quietly means something other than it appears to.

- Batches were awaited in a loop, which runs futures *sequentially*. The
  batching would have looked correct and done nothing.
- Proof attempts run against HEAD while sweeps read the working tree. On a dirty
  repository a real defect in uncommitted code would have been reported as "the
  test written for it passes, so it proves nothing".
- The same mismatch one level up: Kilo reviews a checkout of HEAD while the other
  vendors read the working tree, so a dirty repository meant one run reviewing
  two versions of the code. Both now warn.

## 10. What I did not do

- **No user interface.** Out of scope by instruction.
- **Kilo cannot prove.** Claude and Codex both can now. Kilo is refused with a
  stated reason: it cannot be given an output schema to enforce, so its account
  of what it did cannot be relied on, and a proof step whose self-report cannot
  be trusted is worse than none.
- **Only the correctness lane has ever been run.** Security, Contract and UX
  have written mandates and have never been pointed at anything.
- **No cross-lane severity pass.** The judge merges within a lane only.
- **Detection rate on a real codebase is still unmeasured.**
- **No persistence or resume.** A run that dies starts over.
- **Detection rate on a real codebase is unmeasured.** I measured whether a
  claimed defect can be proved, not what fraction of real defects get found.

## 11. What I would do next, in order

1. **Re-run the kill gate on a real repository.** This is the single most
   valuable next step. The fixture was too easy to separate the vendors, so the
   gate passed on weak evidence. Alder with several bug-fix commits reverted
   would give a real recall number per vendor and a real answer.
2. **Try harder proof cases.** Concurrency, I/O, timing. I expect the success
   rate to drop, and it is better to know by how much before building further on
   the assumption.
3. **Run the other three lanes**, so their mandates are tested rather than just
   written.
4. **Add the proof path for Codex and Kilo**, so proof does not depend on one
   vendor.

## 12. State of the repository

Everything is committed and green. `scripts/verify.ps1` runs the full gate:
formatting, linting with warnings as errors, ~130 tests, a release build, and a
check that no source file exceeds 400 lines.

```bash
pwsh -File scripts/verify.ps1
```
