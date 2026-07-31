# BugSleuth

An adversarial, cross-vendor code review that produces a **ranked,
evidence-backed defect list you can act on without reading the code**.

It exists for a specific problem: shipping code you cannot personally review. An
AI wrote it, you cannot read it, and there is no independent reviewer. Asking one
model to review another model's output has correlated blind spots — especially
within the same family — so BugSleuth asks several different vendors, each with a
different mandate, and then makes them prove it.

**Status: early. Working harness, no user interface.** See
[NIGHT-REPORT.md](NIGHT-REPORT.md) for what has actually been measured, and
[PROGRESS.md](PROGRESS.md) for where to pick up.

## The two ideas

**Findings must carry their own proof.** A well-written hallucination and a real
bug look identical to someone who cannot read the code. So every finding is
checked mechanically: its quoted snippet must exist in the file it names, and
where possible a model is asked to write a test that *fails because of* the
defect. BugSleuth runs that test itself and only believes what it observes.

**Diversity is manufactured, not hoped for.** Each review runs in a *lane* — a
narrow mandate with its own brief — because one generic "find bugs" prompt
collapses toward the same handful of findings whichever model you ask.

## Using it

```bash
cargo run -p bugsleuth-cli -- preflight
```

Checks which provider CLIs can be started. It does **not** prove they are signed
in; only a real sweep does that.

```bash
cargo run -p bugsleuth-cli -- sweep --repo <path> --lane correctness --model sonnet --json-out run.json
```

`--model` takes `vendor:model`. A bare name means Claude. `codex:` and `kilo:`
with nothing after them use each CLI's own default.

```bash
cargo run -p bugsleuth-cli -- judge run-a.json run-b.json run-c.json
```

Merges several sweeps into one ranked list of distinct defects, recording how
many vendors independently found each one.

```bash
cargo run -p bugsleuth-cli -- prove --repo <git repo> --defect-file defect.md --test-command "cargo test"
```

Asks a model to demonstrate a defect with a failing test, in a throwaway git
worktree, then judges the attempt by running the tests independently.

## What it will not do

It does not apply patches, open pull requests, integrate with CI, or chat with
your codebase. It produces a defect list. That is the whole scope, deliberately.

## Safety properties

These are the ones worth knowing, because they are why it can be pointed at a
repository you care about:

- **A review cannot modify the code it reviews.** Codex runs under
  `--sandbox read-only`; Claude runs with an explicit tool allowlist and no write
  tools. Kilo has neither, so its sweeps run in a disposable git worktree — the
  adapter's input type makes the unsafe call impossible to write.
- **The reviewed repository cannot alter its own review.** Every vendor runs with
  its customizations disabled, so a repository's own hooks, agent config or rules
  are not loaded.
- **A lane that failed is never reported as clean.** It says `NOT SWEPT` with the
  reason and exits non-zero. Silently omitting a lane that did not run is the
  most dangerous output this tool could produce.
- **A proof that breaks the code is rejected.** An agent asked to make a test
  fail can always succeed by sabotaging the source. Pass counts are compared
  before and after; if any previously passing test stops passing, the proof is
  thrown out whatever the model claims.
- **API keys are read from the environment only**, never accepted as arguments,
  so they cannot reach a shell history or a process listing.

## Building

```bash
pwsh -File scripts/verify.ps1
```

Formatting, clippy with warnings as errors, the full test suite, a release
build, and a check that no source file exceeds 400 lines.

## Layout

| Crate | Responsibility |
|---|---|
| `bugsleuth-domain` | Lanes, findings, proof verdicts. Types only — no I/O, depends on nothing else here |
| `bugsleuth-provider` | One CLI adapter per vendor, plus shared subprocess handling |
| `bugsleuth-verify` | Anchor checking, git worktrees, test execution |
| `bugsleuth-judge` | Clustering, agreement counting, ranking |
| `bugsleuth-cli` | The `bugsleuth` binary |

Dependencies point one way: everything may depend on `domain`, and `domain`
depends on nothing. `judge` does not know `provider` exists.
