# Night report

Unattended session, 31 July – 1 August 2026. Written for someone who does not
read Rust.

**Two headlines.**

**One: the experiment that decides the project succeeded, and I have the
evidence to back it.** A model asked to prove a defect with a failing test did
so on both real defects it was given, and correctly refused on both fabricated
ones. Section 2 — that is the part worth your attention.

**Two: the cross-vendor premise is not just a theory.** On the same code with
the same instructions, Claude reported 5 defects and Codex reported 9. Codex
found everything Claude found, plus four more — including a real bug that
neither Claude nor I had noticed. Section 4.

Nothing here is production-ready. It is a working harness with two strong
experimental results.

---

## 1. What got built

Four small Rust libraries and a command-line harness. No user interface — that
was explicitly out of scope for an unattended night.

| Piece | What it does |
|---|---|
| `domain` | The vocabulary: lanes, findings, proof verdicts. Pure definitions, no behaviour |
| `provider` | Drives the Claude Code and Codex CLIs as subprocesses, using your signed-in sessions |
| `verify` | Checks findings against reality: does this code exist? Does this test really fail? |
| `bugsleuth` | The command-line harness that ties them together |

Three commands work today:

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
bugsleuth prove --repo <path> --defect-file <file> --test-command "cargo test"
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

## 5. Bugs found in my own work

Worth recording, because they are the kind you could not catch by reading a diff.

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

## 6. Notable things learned from Eir

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

## 7. Quota

Cap for the night was about 40 model invocations. **Used: 7.** One M0 sweep,
four proof attempts, and two Codex sweeps — the first of which failed instantly
on a bad model name and cost nothing. No rate limit was hit.

## 8. What I did not do

- **No user interface.** Out of scope by instruction.
- **Kilo, the third vendor, is not wired up.** Two of three are done.
- **No judge.** With two vendors now producing overlapping findings, merging
  duplicates and normalising severity is the next real piece of work, and it is
  the part the M2 kill gate tests. I did not reach it.
- **No proof step for Codex.** Codex can sweep but cannot yet be asked to prove
  a finding; only Claude can.
- **No persistence or resume.** A run that dies starts over.
- **Detection rate on a real codebase is unmeasured.** I measured whether a
  claimed defect can be proved, not what fraction of real defects get found.

## 9. What I would do next, in order

1. **Build the judge**, and run the M2 kill gate. Two vendors now produce
   overlapping findings on the same code, which is exactly the input a judge
   needs. The gate asks whether the merged top-10 beats the best single model's
   top-10; nothing before this point can answer it.
2. **Measure detection rate properly.** Revert several real bug-fix commits in
   Alder and count how many each vendor finds. Cheap and high-information,
   and now it can be done per vendor.
3. **Try harder proof cases.** Concurrency, I/O, timing. I expect the success
   rate to drop, and it is better to know by how much before building further on
   the assumption.
4. **Add Kilo**, and the Codex proof path.

## 10. State of the repository

Everything is committed and green. `scripts/verify.ps1` runs the full gate:
formatting, linting with warnings as errors, 66 tests, a release build, and a
check that no source file exceeds 400 lines.

```bash
pwsh -File scripts/verify.ps1
```
