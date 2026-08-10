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
domain  ←  provider  ┐
        ←  verify    ├→  engine  ←  cli        (command line)
        ←  judge     ┘           ←  src-tauri  (desktop app)
```

Everything may depend on `domain`. `domain` depends on nothing of ours — no I/O,
no async. `judge` does not know `provider` exists; `provider` does not know
`judge` exists. `cli` is the only crate that composes.

| Crate | Owns | Deliberately does not |
|---|---|---|
| `bugsleuth-domain` | Lanes and their mandates, findings, the JSON schemas | Touch the filesystem or the network |
| `bugsleuth-provider` | One subprocess adapter per vendor; shared spawn/timeout/kill | Know what a lane means, or what a run is |
| `bugsleuth-verify` | Anchor checking, git worktrees, running tests | Know which model produced anything |
| `bugsleuth-judge` | Clustering, agreement counting, ranking | Know how findings were produced |
| `bugsleuth-engine` | Briefs, planning, orchestration, reporting | Contain vendor-specific knowledge, or know which front end called it |
| `bugsleuth-cli` | Argument parsing, printing | Decide anything |
| `src-tauri` | Window, tray, settings, command surface | Contain logic — a command is a deserialize, a call, a serialize |

## The two-type rule

`RawFinding` is what a model claimed. `Finding` is what survived checking. They
are separate types, and the only way to get from one to the other is through
`bugsleuth-verify`. A report holds `Finding`s, so an unverified claim **cannot**
reach a report — not by convention, but because it does not typecheck.

## Where vendor differences live

Entirely inside `provider`, one file per vendor. The differences are real and
absorbing them is most of that crate's job:

| | Schema enforcement | Read-only mechanism | Output |
|---|---|---|---|
| Claude | Inline JSON Schema | Tool allowlist | One JSON envelope |
| Codex | Schema as a **file** | `--sandbox read-only` | Final message to a file |
| Kilo | **None** — described in the prompt | **None** — needs a worktree | NDJSON events, messages repeated |

Three consequences worth knowing:

- **Kilo sweeps run in a throwaway git worktree**, because it is the only way to
  guarantee a review cannot modify the code it is reviewing. The adapter takes a
  `worktree`, not a `repo`, so the unsafe call does not compile.
- **An interrupted provider run is resumed, not restarted.** The shared runner
  kills timed-out process trees but keeps their partial output. Claude receives
  a session id before launch and enables
  `CLAUDE_CODE_RESUME_INTERRUPTED_TURN` on recovery; Codex and Kilo expose their
  ids in early JSON events. Codex transient `turn.failed` events and Kilo's
  native timeout exit 124 take the same path. Each adapter gets one answer-only
  recovery capped at five minutes, and the report marks that result as
  potentially incomplete. With no session id, the lane remains `NOT SWEPT`
  rather than spending another full review from scratch.

Vendor dispatch is an enum, not a trait. The set is closed and small, and the
differences above are worth seeing rather than hiding behind one interface.

## The lane that reviews the checks

Four lanes read `src`. The fifth, **Gate**, reads the test suite, the workflows
and the verification scripts, and hunts one shape: *a check that passes whether
or not the behaviour it names is correct.* It follows from the constraint at the
top of this file — if a claim nobody can check is worthless, so is a check that
cannot fail, and nothing else was looking at those.

It was graded before it shipped, against four seeded gate defects: three sweeps
scored 4/4, 3/4 and 4/4, with no false positives and nothing reported that
belonged to another lane. Then it was run against this repository and found two
real ones — a lane round-trip test whose
`unwrap_or` fallback was the value under test, so the Ux case could not fail,
and a `tsconfig.json` that excluded every `*.test.ts` from the only
type-checking the gate does, so a mismatch between a function and its own test
passed everything. Both are fixed; both had been read over many times.

## The path a run takes

1. **Plan** — the config assigns lanes to models; the (model × lane) product is
   enumerated. Every lane is always listed, so one with no model assigned is
   carried through as an explicit gap.
2. **Desktop pre-check** — the app gives each selected provider one minimal real
   invocation, concurrently. Missing sessions, unhealthy CLIs and an unsafe
   Kilo `ask` policy stop the desktop run before any lane starts.
3. **Batch** — different vendors run together, but each vendor's sweeps run one
   at a time. Claude, Codex and Kilo all support parallel agents in some product
   surfaces, but none publishes a safe maximum for independent authenticated
   CLI processes. Two Kilo processes were observed colliding while updating its
   shared credential database, so BugSleuth does not guess a limit.
4. **Sweep** — each unit runs its vendor against the repository. Failure is a
   *reported state*, never an exception that vanishes.
5. **Verify** — every finding's quoted snippet must exist in the file it names,
   or it is discarded. Line numbers are corrected rather than treated as fatal.
6. **Judge** — findings are clustered by anchor *and* wording; agreement is
   counted per distinct model; the result is ranked severity-first. The ranked
   list, its gaps and a fix prompt are the run's output — handed to a model to
   fix, or read as-is.

## Two front ends, one engine

The engine is a library rather than living inside the command-line binary
specifically so the desktop app runs the same code. Two implementations of "is
this lane swept or merely empty" would be exactly the quiet divergence this tool
exists to catch in other people's code.

The desktop shell adds only what a window needs:

- **No filesystem or shell permission is granted to the frontend.** The only way
  to touch disk is a command that canonicalizes and checks the path first. The
  capability file lists the handful of window calls actually used, nothing more.
- **The window starts hidden** with a background matching the dark theme, and
  the frontend reveals it after mounting, so there is no unstyled flash.
- **Closing hides to the tray**, and `prevent_close` runs before anything
  fallible. The tray's Quit is the single real exit path.
- **Theme "match system" removes the attribute** rather than resolving it in
  JavaScript, so the app follows the OS live instead of freezing the choice at
  startup.
- **The tray icon is a different drawing from the app icon**, not a scaled copy.
  At 16-24px the app icon's legs and highlight collapse into noise; an early
  single-icon version read as an asterisk.

## The invariants that matter

These are the ones to protect when changing anything:

- **A lane that did not run is never rendered as a lane that found nothing.**
  Both kinds of hole — no model assigned, and sweep failed — are named with a
  reason, and either makes the command exit non-zero.
- **Cross-lane severities are compared only after a complete triage pass.**
  That pass grades the merged list against one rubric. If it is disabled,
  fails, or is partial, the report warns that lane-relative grades are not
  comparable.
- **A review cannot modify the code it reviews**, and **the reviewed repository
  cannot alter its own review** (every vendor runs with customizations disabled,
  so the target's hooks and config are not loaded).

## Enforced mechanically

- **One gate.** `scripts/verify.sh` is the whole of it; `scripts/verify.ps1` is a
  shim that runs it through Git for Windows' Bash. There were two, kept in step
  by hand, and they disagreed three times in a single day — different line
  counts, different file types, different build outputs, the last of which
  published a release on one platform only. The only reliable parity is having
  nothing to mirror.
- **A pre-push hook runs that gate**, installed by `scripts/install-hooks.sh`. A
  person remembering to read an exit code is not a control: the gate caught a
  file over the cap and the commit reached the remote anyway, because the
  command piped it into `tail` and a pipeline exits with its last stage's
  status.
- 400-line hard cap on every `.rs`, `.ts`, `.tsx`.
- **`tests.lock`** — the name of every test. A test that stops running fails the
  build. Splitting files under the line cap lost tests twice in one day and the
  count did not move either time, because tests were added in the same change.
- **The frontend's checks read TypeScript's own syntax tree**, not regular
  expressions. Six defects in one day were a regex over source that matched less
  than existed and returned a smaller answer instead of an error.
- **Names that cross the JavaScript/Rust boundary are compared**: commands,
  events, settings fields and lane titles. Each is a string on both sides with
  no compiler spanning them.
- **Shared prose lives in one function.** The review limits, the unsandboxed
  caution and the cut-short note are each written once and asserted to appear in
  every document that reports findings.
- `unsafe_code = "forbid"` workspace-wide; a crate needing it must opt out and
  say why.
- The frontend type-checks under `strict` with `noUncheckedIndexedAccess`, and
  its state rules have their own tests, so the gate covers the app rather than
  half of it.
- `clippy` with warnings as errors, including `too_many_lines` and
  `cognitive_complexity`.
- Crate boundaries, which the compiler enforces for free.

## Applying the fixes

The window can hand the report straight back to a model, which is the one thing
in BugSleuth that writes to the repository. It runs the same `fix-prompt.md` the
Copy button gives you, read from disk rather than from the window — the argument
that becomes instructions to an agent with write access should not be a string
the webview chose.

Every vendor can apply, and what each is confined by differs. None of it bounds
a shell, so none of it is a sandbox in the sense that word usually carries:

- **Claude** grants `Edit` and `Write` as `./**` rules, so the repository is the
  only tree it writes to without asking — and it has no way to ask.
- **Codex** runs under `--sandbox workspace-write`, which the kernel enforces on
  macOS and Linux. Windows has no such enforcement in the CLI.
- **Kimi** is confined by its agent file, whose `tools` list is an allowlist;
  omitting it would allow every tool, including the ones that spawn subagents.
- **Kilo** has no per-invocation limit at all — its permissions come from the
  machine's own config. So an apply is refused outright when the repository
  ships Kilo configuration of its own, because a `kilo.jsonc` in the working
  directory rewrites the resolved permissions of the agent named on the command
  line: the repository would be choosing what the model applying its fixes may
  do.

Beyond that the guarantee is git:

- **The working tree must be clean**, or it is refused. Your uncommitted work
  and the model's changes would otherwise be one indistinguishable pile.
- **What changed is reported from git, never from the model.** Compared against
  the commit the repository started on, so a fix the model *committed* still
  counts — `git status` alone would show a clean tree and read as "nothing
  happened".
- A sweep and an apply may not overlap, enforced in the shell rather than by
  disabling a button: a review reading the tree while an apply rewrites it
  reports on code that no longer exists.

One vendor difference is worth knowing, because it was silent: Codex refuses
every write when `--ignore-user-config` is set, whatever `--sandbox` says, and
no config override restores it. The apply invocation therefore drops that flag —
it is the only invocation that has to write.

## Not built, deliberately

No PRs or CI integration. No persistence beyond
per-sweep JSON files — `--resume` reads those rather than a database, which is
the smallest thing that makes a dead run recoverable.
