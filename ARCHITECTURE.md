# BugSleuth — Architecture

Living document. Code is ground truth; correct this when they diverge.

**Last updated:** 1 August 2026.

## The constraint everything follows from

BugSleuth's reader cannot check a finding by reading the code. To them a
confident hallucination and a real defect look identical. Every structural
decision below exists to make claims *mechanically* checkable, or to make it
impossible to present an unchecked claim as a checked one.

That is also why the layering is enforced by the compiler and the line limits by
a script: a rule the owner cannot verify by reading has to be a rule the build
checks.

## Crates, and which way dependencies point

```
domain  ←  provider
        ←  verify
        ←  judge
        ←  cli  (composes all of them)
```

Everything may depend on `domain`. `domain` depends on nothing of ours — no I/O,
no async. `judge` does not know `provider` exists; `provider` does not know
`judge` exists. `cli` is the only crate that composes.

| Crate | Owns | Deliberately does not |
|---|---|---|
| `bugsleuth-domain` | Lanes and their mandates, findings, proof verdicts, the JSON schemas | Touch the filesystem or the network |
| `bugsleuth-provider` | One subprocess adapter per vendor; shared spawn/timeout/kill | Know what a lane means, or what a run is |
| `bugsleuth-verify` | Anchor checking, git worktrees, running tests | Know which model produced anything |
| `bugsleuth-judge` | Clustering, agreement counting, ranking | Know how findings were produced |
| `bugsleuth-cli` | Briefs, orchestration, reporting | Contain vendor-specific knowledge |

## The two-type rule

`RawFinding` is what a model claimed. `Finding` is what survived checking. They
are separate types, and the only way to get from one to the other is through
`bugsleuth-verify`. A report holds `Finding`s, so an unverified claim **cannot**
reach a report — not by convention, but because it does not typecheck.

The same shape appears in proof: `ProofClaim` is the model's account of what it
did; `ProofVerdict` is what we observed by running the tests ourselves.

## Where vendor differences live

Entirely inside `provider`, one file per vendor. The differences are real and
absorbing them is most of that crate's job:

| | Schema enforcement | Read-only mechanism | Output |
|---|---|---|---|
| Claude | Inline JSON Schema | Tool allowlist | One JSON envelope |
| Codex | Schema as a **file** | `--sandbox read-only` | Final message to a file |
| Kilo | **None** — described in the prompt | **None** — needs a worktree | NDJSON events, messages repeated |

Two consequences worth knowing:

- **Kilo sweeps run in a throwaway git worktree**, because it is the only way to
  guarantee a review cannot modify the code it is reviewing. The adapter takes a
  `worktree`, not a `repo`, so the unsafe call does not compile.
- **Kilo cannot prove.** Without enforceable structured output its self-report
  cannot be relied on, and a proof step that launders a guess into apparent
  evidence is worse than no proof step.

Vendor dispatch is an enum, not a trait. The set is closed and small, and the
differences above are worth seeing rather than hiding behind one interface.

## The path a run takes

1. **Plan** — the config assigns lanes to models; the (model × lane) product is
   enumerated. Every lane is always listed, so one with no model assigned is
   carried through as an explicit gap.
2. **Batch** — units are grouped so no two invocations of the same vendor run at
   once. With CLI subscriptions the binding constraint is rate limits, not money.
3. **Sweep** — each unit runs its vendor against the repository. Failure is a
   *reported state*, never an exception that vanishes.
4. **Verify** — every finding's quoted snippet must exist in the file it names,
   or it is discarded. Line numbers are corrected rather than treated as fatal.
5. **Judge** — findings are clustered by anchor *and* wording; agreement is
   counted per distinct model; the result is ranked severity-first.
6. **Prove** (optional) — the top N defects get a failing-test attempt in a
   throwaway worktree, judged by re-running the tests ourselves.

## The invariants that matter

These are the ones to protect when changing anything:

- **A lane that did not run is never rendered as a lane that found nothing.**
  Both kinds of hole — no model assigned, and sweep failed — are named with a
  reason, and either makes the command exit non-zero.
- **A proof that broke the code is rejected.** Pass counts are compared before
  and after; if any previously passing test stops passing, the attempt is thrown
  out regardless of what the model claims. An agent asked to make a test fail can
  always succeed by sabotage, and that produces a red test proving nothing.
- **"Not attempted" is never "not proven."**
- **Severities are not compared across lanes.** They were assigned by models
  answering different questions.
- **A review cannot modify the code it reviews**, and **the reviewed repository
  cannot alter its own review** (every vendor runs with customizations disabled,
  so the target's hooks and config are not loaded).

## Enforced mechanically

- `scripts/check-file-size.ps1` — 400-line hard cap, 300 soft, on every `.rs`,
  `.ts`, `.tsx`. Part of `scripts/verify.ps1`.
- `clippy` with warnings as errors, including `too_many_lines` and
  `cognitive_complexity`.
- Crate boundaries, which the compiler enforces for free.

## Not built, deliberately

No UI. No patch application, PRs, or CI integration. No persistence beyond
per-sweep JSON files — `--resume` reads those rather than a database, which is
the smallest thing that makes a dead run recoverable.
