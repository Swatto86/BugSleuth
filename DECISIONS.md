# Decisions

One entry per decision: what was decided, what else was considered, why, and how
to reverse it. Written for someone who does not read Rust.

---

## Part 1 — What Eir taught us

Eir at `C:\Users\Swatto\eir` was read as reference only. Nothing in it was
modified. It already solves the hardest unsolved part of this project: driving
the Codex, Claude and Kilo CLIs using sessions you are already signed into, with
no API keys.

### 1.1 How each CLI is invoked

All three are spawned as ordinary child processes from Rust — **no shell, no
Tauri sidecar, no `tauri-plugin-shell`**. The prompt goes in on **stdin**, not as
a command-line argument.

| CLI | Arguments Eir uses |
|---|---|
| Claude | `--print --output-format json` plus `--model X` and `--effort X` when set |
| Codex | `--ask-for-approval never exec --json --sandbox read-only --skip-git-repo-check --ephemeral --ignore-user-config --ignore-rules --color never` plus `-m X`, `-c model_reasoning_effort=X`, and a trailing `-` meaning "read the prompt from stdin" |
| Kilo | `run --auto --format json --agent ask` plus `-m X` and `--variant low\|medium\|high` |

Working directory: Claude runs wherever the service happens to be; Codex and Kilo
are given a throwaway scratch directory that is deleted afterwards. Environment:
Eir only overrides `USERPROFILE`/`HOME`/`APPDATA`/`LOCALAPPDATA`, and only when
borrowing another user's session.

**One thing Eir does that BugSleuth must not copy.** Eir runs as a Windows
service under the SYSTEM account, so it has an elaborate impersonation path to
drop down to your user account before running a CLI. BugSleuth is an ordinary
desktop app running as you, so all of that is unnecessary — roughly 400 lines of
Eir's CLI code does not apply to us at all.

### 1.2 How it detects a CLI is installed and signed in

**It does not.** Eir has no pre-flight check. It searches a hard-coded list of
likely install paths, falls back to the bare command name, tries to run it, and
if that fails, reports a helpful error message ("is it installed and logged in?").

That is a reasonable choice for Eir, which makes one call at a time. It is a poor
choice for BugSleuth, where a sweep is a long queue of calls and discovering a
signed-out CLI on call number twelve wastes everything the user already waited
for. **BugSleuth adds a preflight command** — see decision 2.6.

### 1.3 How output is parsed

Buffered, not streamed. Eir waits for the process to finish, then parses.

- **Claude** returns a single JSON object — the transcript envelope — with the
  model's actual reply in a `result` field, plus cost and token counts. If that
  envelope does not parse, Eir treats the whole output as plain text.
- **Codex** returns newline-delimited JSON events. Eir walks them for
  `item.completed` events carrying an `agent_message` and concatenates the text,
  and reads token counts from `turn.completed`.
- **Kilo** also returns newline-delimited JSON events, with a different shape
  again: payloads nested under a `part` object, text arriving in `text` events,
  and accounting in the last `step_finish` event.

So the three schemas are genuinely different, and the differences are messy
enough that Eir carries defensive fallbacks for older event shapes in the Kilo
parser. **This is the strongest argument for keeping a hard adapter boundary**:
the ugliness must not leak upward.

### 1.4 Lifecycle

- 300-second timeout on every call; the child is killed on expiry.
- 16 MB cap per output stream — it keeps reading past the cap but stops storing,
  so the child never blocks on a full pipe while memory stays bounded.
- stdin is written *concurrently* with reading stdout/stderr. Eir has a comment
  recording that doing it sequentially deadlocked in production once the prompt
  exceeded the operating system's ~64 KB pipe buffer. **This is the single most
  valuable thing in the file** and BugSleuth copies it deliberately.
- `kill_on_drop` so an abandoned call does not leave an orphan process.
- Non-zero exit with no error text is treated as a *transient* failure worth
  retrying, because the CLIs swallow overload blips that way.

### 1.5 Tauri wiring

**There is none, and that is the finding.** Eir's Tauri app never launches a CLI.
The Windows service does, from plain Rust. The Tauri app uses only the updater,
notification, autostart, single-instance and dialog plugins, and has a single
`capabilities/main.json`.

This is good news: BugSleuth does not need `tauri-plugin-shell`, does not need
sidecar binaries in `tauri.conf.json`, and does not need shell-scope entries in
its capabilities file. Spawning from Rust avoids the plugin's permission model
entirely and is both simpler and tighter.

### 1.6 What transfers

| Transfers unchanged | Needs extending | Does not apply |
|---|---|---|
| Concurrent stdin/stdout pumping | Timeouts (300 s is far too short for an agentic repo sweep) | The whole SYSTEM-impersonation layer |
| Capped stream reading | Output parsing (Eir wants prose; we want schema-validated findings) | Scratch-directory workspaces (we use git worktrees) |
| Binary path discovery | Concurrency (Eir does one call; we need many, governed) | HTTP/OpenRouter client paths |
| `kill_on_drop`, timeout-then-kill | Detection (Eir has none) | Its cost accounting model |
| Treating silent non-zero exit as transient | Resumability (Eir has none — a dead call is just lost) | |

Reusable as a shared crate? **Not worth it.** The genuinely common part is about
120 lines of process handling. Extracting a crate shared between two repositories
would couple Eir's release cycle to BugSleuth's for very little. Copied
deliberately, with the reasoning preserved in comments. Reverse by extracting
later if a third consumer appears.

---

## Part 2 — BugSleuth's own decisions

### 2.1 Three crates now, not six

**Decided:** build `domain`, `provider` and `verify` plus a harness binary. Do
not create `engine`, `judge` or `store` yet.

**Why:** an empty crate is not a boundary, it is a guess about where a boundary
will go. All three of the deferred crates belong to milestone M2, and M2 only
happens if M1 succeeds. Creating them now means designing their interfaces before
seeing any evidence.

**Reverse:** adding a crate later is a five-minute mechanical change, and the
compiler finds every affected line.

### 2.2 No provider trait yet

**Decided:** write the Claude adapter as concrete code. Do not define a
`Provider` interface until there is a second adapter.

**Why:** a trait designed from one example is a guess about what the three CLIs
have in common — and section 1.3 shows their output schemas differ substantially.
Designing the shared interface *after* seeing all three real behaviours will
produce a better one. This is the reversible choice; the opposite is not.

**Reverse:** introduce the trait when Codex or Kilo is added. The compiler
enumerates every call site.

### 2.3 Spawn the real `claude.exe`, never the npm `.cmd` shim

**Decided:** find and run `claude.exe` directly.

**Why (security):** on Windows, the npm-installed `claude.cmd` is a batch file
and must be run through `cmd.exe`. That would re-expose every argument to shell
parsing — and one of our arguments is a JSON Schema full of quotes and braces.
Spawning the executable directly passes arguments as a list, with no shell
anywhere in the path.

**Reverse:** if a machine has only the shim, this fails with a clear "not found"
message rather than silently doing something unsafe. That is the intended
trade-off.

### 2.4 Run the review with `--safe-mode`

**Decided:** every sweep passes `--safe-mode`.

**Why:** without it, reviewing a repository would load *that repository's*
`CLAUDE.md`, hooks, skills, custom agents and MCP servers. Two problems. First,
security: reviewing an untrusted repository would execute code it supplied.
Second, reproducibility: the review's behaviour would silently depend on whatever
is configured on the machine, so two people would get different reports from the
same code. `--safe-mode` disables all of it while leaving authentication alone,
so the signed-in subscription session still works.

**Rejected alternative:** `--bare`, which does the same job but *forces* API-key
authentication and so cannot use the subscription at all.

### 2.5 An explicit tool allowlist, not `--dangerously-skip-permissions`

**Decided:** a read-only sweep is given exactly `Read`, `Glob`, `Grep`, and is
explicitly denied `Edit`, `Write`, `NotebookEdit`, `Bash`, `WebFetch`,
`WebSearch`.

**Why:** a review has no business modifying the code it reviews, and a sweep that
*cannot* write is a much stronger guarantee than one that is merely asked not to.
Proof-carrying findings (M1) genuinely need write and test-execution access —
that is a separate, wider permission set used inside a throwaway git worktree,
never against your working tree.

### 2.6 A preflight command

**Decided:** `bugsleuth preflight` checks the CLI can be found and run, before
any sweep starts. Eir has no equivalent.

**Why:** a sweep is a long queue. Discovering a missing CLI at the end wastes the
whole wait.

**Honest limitation, stated in the code:** `--version` succeeds for a CLI that is
installed but *not signed in*. Only a real call proves authentication, so a run
still has to handle an auth failure when it happens. Preflight catches the common
case cheaply and for free; it is not a guarantee.

### 2.7 Line numbers are corrected, not treated as fatal

**Decided:** a finding must quote code that appears **verbatim somewhere in the
file it names**, or it is discarded. But if the quoted code is found at a
different line than claimed, the line number is corrected and the finding kept.
The correction is recorded and shown in the report.

**This is a deliberate deviation from the brief**, which said to drop a finding
whose snippet does not match at the stated location.

**Why:** the property that matters is "this model is describing code that really
exists", and quoting is what tests that. Line numbering is a separate, much
weaker skill — models routinely miscount by a few lines while quoting perfectly.
Dropping on exact line equality would discard genuine defects for a reason that
has nothing to do with whether the defect is real, which would throw away most of
the filter's value while keeping all of its cost.

Two safeguards keep this honest: the quote must match *exactly* (ignoring only
indentation), and when a snippet legitimately appears several times, the
occurrence **nearest the claimed line** is chosen — so the line number still
carries information, it just is not a veto.

**Reverse:** one comparison in `crates/bugsleuth-verify/src/anchor.rs`.

**Measured result:** on the first real sweep, 5 of 5 findings verified and 0 were
discarded, so this leniency was not what let anything through. See
`NIGHT-REPORT.md`.

### 2.8 A failed lane is a reported state, never an error

**Decided:** when a sweep fails, the report says `NOT SWEPT` with the reason. It
never renders as "no findings". The command also exits with code 2 so a script
cannot mistake it for a clean pass.

**Why:** this is the most dangerous failure this tool could have. A report that
quietly omits a lane that never ran reads exactly like a report where that lane
found nothing — and you would act on it.

### 2.9 The answer key lives outside the fixture repository

**Decided:** `fixtures/seeded-repo` contains the seeded bugs; the list of what
they are lives in `fixtures/SEEDED.md`, one level up.

**Why:** a sweep is pointed at `fixtures/seeded-repo` as its working directory,
so it cannot read a file above that. If the answer key were inside, the
measurement would be worthless.

### 2.10 The API key is read from the environment, never from an argument

**Decided:** `--use-api-key` is a flag; the key itself is only ever read from
`ANTHROPIC_API_KEY`.

**Why:** a key passed as a command-line argument appears in shell history and in
the process list, where any other process on the machine can read it.

### 2.11 File-size limits are enforced by a script, not by review

**Decided:** `scripts/check-file-size.ps1` fails the build if any `.rs`, `.ts` or
`.tsx` file exceeds 400 lines, and warns over 300. It runs as part of
`scripts/verify.ps1`.

**Why:** you cannot spot an 900-line file by reading a diff. A rule that cannot
be verified by reading has to be a rule the build checks.
