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

### 2.12 Vendor dispatch is an enum, not a trait

**Decided:** picking between Claude and Codex is a plain `match` on a two-case
enum. There is no `Provider` interface.

**Why:** the brief called for a trait, and decision 2.2 deferred it until a
second adapter existed so it could be designed from evidence rather than from
one example. The second adapter now exists, and the evidence argues against the
trait. The set of vendors is closed and small — three CLIs we ship support for
ourselves — so a trait would buy extensibility nobody needs. It would also hide
the differences between adapters behind a uniform surface, and those differences
(schema as a file versus inline, sandbox flag versus tool allowlist) are exactly
what a reader needs to see.

**Reverse:** introduce the trait when a fourth vendor appears or when the engine
genuinely needs to hold a heterogeneous list. The compiler enumerates every call
site.

### 2.13 Codex runs read-only via its sandbox, not a tool allowlist

**Decided:** Codex sweeps pass `--sandbox read-only`; Claude sweeps use an
explicit tool allowlist. Different mechanisms, same guarantee.

**Why:** each vendor's own mechanism is the stronger one on that vendor. Codex's
sandbox is enforced below the agent — the operating system refuses the write —
which is a better guarantee than asking the agent not to try. Claude has no
equivalent sandbox flag, so the allowlist is its strongest available control.
Forcing both into one shape would weaken whichever one got the worse fit.

Both also disable the reviewed repository's own configuration: `--safe-mode` for
Claude, `--ignore-user-config --ignore-rules` for Codex.

### 2.14 One error type for the whole provider crate

**Decided:** a single `ProviderError` with a `vendor` label, rather than one
error type per adapter.

**Why:** the engine above only needs to know which of a small number of things
went wrong and whether retrying is worth it. Per-vendor error types would push
that decision upward and duplicate it. `is_transient` lives here so the retry
policy is written once — a silent non-zero exit is how these CLIs report an
overload blip, and that judgement should not be re-derived per adapter.

**Introduced when the second adapter arrived, not before.** Sharing designed
from one example is a guess.

### 2.15 The seeded fixture keeps a bug the tool found and I missed

**Decided:** `parse_price("12.3")` returns 1203 pence instead of 1230. Codex
reported it; it was not planted. It stays, and the answer key records that the
tool found it.

**Why:** a fixture curated to contain exactly what the tool already finds cannot
measure the tool. Leaving a defect that the fixture's own author missed is a
more honest test, and it is direct evidence for the cross-vendor premise —
Claude swept the same file and did not report it.

### 2.16 The judge is arithmetic, not another model

**Decided:** merging duplicate findings is done by comparing anchors and wording
in plain code. No model is asked to adjudicate.

**Why:** the brief suggested a model judge on a BYOD API key. Before spending
quota on that, the cheap version had to be beaten. Deterministic clustering is
free, instant, reproducible, and — crucially — *testable against real data*,
which a model judge is not. On hand-labelled real cross-vendor output it merges
the same defect described by two vendors and keeps genuinely different defects
on adjacent lines apart.

A model judge may still be worth adding for the harder cases. If it is, it
should be **measured against this baseline**, not assumed to be better. That is
the whole reason to build the cheap one first.

**Reverse:** add a model adjudication step after clustering, for pairs that
score near the threshold. The current code is what tells you whether that is
worth it.

### 2.17 The merge threshold was measured, not chosen

**Decided:** two findings merge at a wording overlap of 0.20 or above.

**Why:** the first attempt used a guessed 0.25 and silently reported the same
defect twice, because `average_price` and "the average price" shared no words
and "remove_stock underflows" did not match "Removing more stock ... underflows".
Both were fixed by measuring: splitting identifiers into their parts, and crude
suffix stripping so grammar does not hide a match.

Against hand-labelled real output, same-defect pairs then scored 0.24 to 0.32
and different-defect pairs on adjacent lines scored 0.07 to 0.08. 0.20 sits in
that gap.

**The direction of error is deliberate.** Erring toward *not* merging leaves a
duplicate in the report, which is an annoyance. Erring the other way silently
collapses two distinct defects into one, and the reader never learns the second
exists. For a tool whose reader cannot check the code, those are not
symmetrical.

**Reverse:** one constant, and the integration test that pins it says what
moving it will break.

### 2.18 Kilo sweeps run in a throwaway worktree; the other two do not

**Decided:** a Kilo sweep is given a disposable git worktree and never the
repository under review. Claude and Codex are pointed at the repository directly.

**Why:** this is not a preference, it is a difference in what each CLI can be
made to guarantee. Codex takes `--sandbox read-only`, where the operating system
refuses the write. Claude takes a tool allowlist, so the write tools are simply
absent. **Kilo has neither.** Its permissions come from the user's own global
config, and on this machine both candidate agents were configured to allow
everything, with no per-invocation override.

So the only way to guarantee a Kilo review cannot modify the code it is
reviewing is to hand it a copy. The adapter enforces this rather than trusting
the caller: its input type is a `worktree`, not a `repo`, so the unsafe call
does not compile.

**Cost:** a Kilo sweep needs the target to be a git repository, and fails with a
clear message when it is not. That is the right failure — the alternative is
running a review with write access to your working tree.

**Reverse:** if Kilo gains a read-only flag, drop the isolation for it too.

### 2.19 Kilo gets the schema in its prompt, which is weaker and is labelled so

**Decided:** the brief text differs by vendor. Claude and Codex are handed a JSON
Schema the CLI enforces; Kilo, which has no such flag, gets the schema written
into the prompt instead.

**Why:** there is no alternative — but the difference is real and should not be
hidden. A schema is a constraint; a prompt is a request. Expect a higher rate of
unusable replies from Kilo, and read a Kilo failure as "malformed output" before
concluding "found nothing".

The first real Kilo sweep failed exactly this way, though for a different reason
(see 2.20).

### 2.20 Two defects only running the tools could have found

Recorded because they argue for a working practice, not just because they were
bugs.

**Kilo repeats itself.** Each `text` event carries the *complete* text of its
message, not an incremental piece, and the same message is emitted more than
once. Concatenating them — the obvious reading, and what Eir's parser does —
produced two valid JSON objects run together into one invalid document.

**Worktrees survived their own cleanup.** Once `cargo test` has run inside one,
its `target/` paths exceed the Windows path limit, `git worktree remove` gives up
with "Filename too long", and the directory stays. The consequence was not
cosmetic: leftovers made the *reviewed repository* dirty, which is precisely what
BugSleuth promises not to do, and would have broken the clean-baseline check the
next proof attempt depends on.

Neither was visible by reading the code, and neither would have been caught by a
test written from the documentation. Both came from running the thing and then
checking what it left behind. **Check the repository afterwards, every time.**

---

## Part 3 — The desktop app

### 3.1 The engine became a library before the UI existed

**Decided:** move briefs, planning, sweeping, merging and proving out of the
command-line binary into `bugsleuth-engine`, and make both front ends call it.

**Why:** a Tauri app cannot import from a binary crate, so the alternative was
reimplementing orchestration in the app. Two implementations of "is this lane
swept or merely empty" is exactly the quiet divergence this tool exists to catch
in other people's code — it would be embarrassing to ship it in this one.

### 3.2 The frontend gets no filesystem or shell permission at all

**Decided:** the Tauri capability file grants `core:default`, four window calls,
event listening, and a folder picker. Nothing else.

**Why:** everything BugSleuth does that touches disk or spawns a process happens
in Rust behind a command that validates first. Granting the webview `fs` scope
would create a second door with weaker checks, for no benefit — the frontend
never needs to read a file.

A repository path still arrives as a string the frontend chose, so it is
canonicalized and checked to be a real directory before anything acts on it, and
the Windows extended-length prefix is stripped because git rejects it.

### 3.3 The tray exists because a sweep is slow, not because tray icons are nice

**Decided:** BugSleuth is resident. Closing the window hides it; the tray menu
and an in-app button both quit.

**Why:** a run is tens of minutes. The realistic use is to start one, close the
window, and be told when it lands. A tray icon on an app you interact with for
ten seconds at a time would be clutter; here it is the whole point.

**Two exits, deliberately.** The close button hides, so without a second path the
tray would be a single point of failure — and a tray icon that fails to appear
is a real Windows failure mode. An app you can only end from the task manager is
precisely the unrecoverable state the UX lane's mandate describes.

### 3.4 Two icons, not one scaled down

**Decided:** `app-icon.svg` and `tray-icon.svg` are different drawings.

**Why:** measured, not assumed. The first version was one icon with legs,
antennae and a specular highlight. At 128px it read well; at 32px the legs
reached the rim and the whole thing read as an asterisk or a sun, and the thin
handle disappeared. The tray version drops every one of those details and keeps
only the ring, a fat handle and a solid body.

Both were checked by generating the PNGs and looking at them at the sizes that
matter, and the shipped icon was checked by extracting it *from the built
executable* rather than trusting the window.

### 3.5 Theme "match system" removes the attribute rather than resolving it

**Decided:** choosing "match system" deletes `data-theme` from the document so
the CSS media query takes over. It is not resolved to light or dark in
JavaScript.

**Why:** resolving in JS freezes the choice at whatever the OS was doing when the
app started. Letting CSS handle it means the app follows a change made while it
is running, which is what "match system" means.

### 3.6 Windows needed an 8 MB main-thread stack

**Decided:** `.cargo/config.toml` passes `/STACK:8388608` on the MSVC target.

**Why:** the release build overflowed its stack on startup and exited before
showing a window, while the debug build was fine. Windows gives the main thread
1 MB; with LTO, Tauri's generated context initialiser — which embeds the whole
frontend bundle — becomes one very deep frame.

**Worth recording because the symptom is so misleading:** an app that exits
instantly with "has overflowed its stack" looks like infinite recursion, and
there is none. The trigger is the optimisation profile, so it appears only in
release, which is the build you ship.

### 3.7 What the app has been shown to do, and how

Both gaps this section used to record are now closed, and closed by clicking
rather than by reading the code:

- **A real review has been run from the app.** Progress streamed into the window
  and the sweep's JSON landed in `%APPDATA%\BugSleuth\runs`, naming the model
  and carrying anchors that resolve to real lines of the fixture. That last part
  is the evidence that counts: the window could display anything, but only a
  real run writes that file.
- **The tray was driven.** Closing the window hid it and left the process alive;
  the tray icon restored it with its state intact; the menu offered Open and
  Quit; Quit exited cleanly with no orphan.

The journey is written down in [RUNBOOK.md](RUNBOOK.md) so it can be repeated
rather than rediscovered. It was driven manually, because the WebDriver harness
does not observe anything — see 3.10.

What remains unobserved is narrower and worth naming: **proving has never been
run from the app.** It is wired, it refuses clearly when the repository is not
a git checkout or no test command is set, and the engine underneath is the same
code the experiments used — but no proof round has been started by clicking
Run.

### 3.8 The portable executable needed the VC++ runtime, and no longer does

**Observed, then fixed.** `dumpbin /dependents` on the release binary showed no
`WebView2Loader.dll` — good, that one is handled — but it does import
`VCRUNTIME140.dll` and `VCRUNTIME140_1.dll`. Those come from the Visual C++
Redistributable, which is very commonly present but is *not* part of a clean
Windows install.

Core's rule is that a published desktop tool ships at least one self-contained,
directly-runnable binary that launches with no installer and no prerequisites.
As it stands the portable exe would fail on a machine without the redistributable
— and it would fail with an unhelpful system dialog, not a message from us.

**Fixed** with `-C target-feature=+crt-static` in `.cargo/config.toml`. The
binary now imports nothing outside `System32`: the `VCRUNTIME140*` pair is gone
and so are the `api-ms-win-crt-*` UCRT imports, since those link statically too.
Size was unchanged at 9.0 MB, and the app was re-launched afterwards to confirm
static linking had not broken the webview.

**Worth knowing: this is not a Tauri problem and not unusual.** Every Rust
binary built for `x86_64-pc-windows-msvc` links the MSVC runtime dynamically by
default — the CLI in this repo had the same import. Eir already carries exactly
this setting in its own `.cargo/config.toml`, which is why its binaries are
self-contained and the symptom never appeared there. BugSleuth simply started
from an empty workspace and did not inherit it.

**Still worth doing before a release:** run the portable exe on a machine without
the redistributable. Absent imports are good evidence, not proof that it starts.

### 3.9 `cargo build --release` does not produce a shippable Tauri app

**Learned the hard way, and worth more than the fix.**

A binary built with `cargo build --release` embeds the *dev-server URL* rather
than the frontend. With Vite running it looks perfect. Without it the window is
blank — and every check available short of looking at the pixels passes:

- `cargo build` exits 0.
- The process starts and stays up.
- A window exists, is titled correctly, and is visible.
- The icon extracted from the executable is right.

Only `cargo tauri build` bundles the frontend in. `~/.agents/tauri.md` says this
outright — "a direct Rust build is not a packaged-app check" — and it was read at
the start of this work and then not followed, because `cargo build --release`
*looked* like it was producing the same artefact.

**Three traps around it, all of which cost time:**

1. **Cargo caches the decision, in the wrong package.** Tauri's *own* build
   script decides dev-vs-production and emits `DEP_TAURI_DEV`. Once a plain
   `cargo build --release` has recorded `true`, every later `cargo tauri build`
   reuses it — the build log shows `DEP_TAURI_DEV=true` and
   `cargo:rustc-cfg=dev` even with `PROFILE=release` and `DEBUG=false`.

   **`cargo clean -p bugsleuth-app` does not fix it**, because the poisoned
   cache belongs to the dependency, not to the app. It needs
   `cargo clean --release`.

   The gate now avoids creating the problem at all: the fast path builds the
   workspace `--exclude bugsleuth-app`, and `-Package` does a full clean before
   `cargo tauri build`.
2. **`npx tauri build` is not the same command** as `cargo tauri build`. Without
   the npm CLI package installed it fails with "could not determine executable
   to run" — quietly enough to look like a successful no-op.
3. **You cannot tell the two apart by inspection.** Tauri compresses the
   embedded assets, so grepping the binary for markup finds nothing either way,
   and the config it embeds contains `devUrl` in *both* kinds.

**The reliable test is behavioural.** Launch the binary with no dev server and
check for an effect the frontend causes — this app writes its settings file once
the UI mounts, so the file appearing is proof the frontend ran. That check takes
ten seconds and is worth more than every static inspection above combined.

### 3.10 WebDriver drives the app but never sees its page

**Status: unresolved, and recorded rather than papered over.**

`tauri-driver` 2.0.6 with a version-matched Edge driver does create a session
and does launch the app — the process appears about four seconds in. But the
webview WebDriver attaches to is `about:blank` with an empty document, and the
app's frontend never runs while automation is attached.

What was ruled out, each by experiment rather than reasoning:

- **Not a broken app.** Launched normally with no dev server, the frontend runs:
  it writes its settings file, which only the mounted UI does.
- **Not the hidden window.** Rebuilt with `visible: true` and the result was
  identical, so the reveal path is not implicated.
- **Not a stale or dev binary.** The same binary that fails under automation
  passes the standalone behavioural check.
- **Not a version mismatch.** The Edge driver is matched exactly to the
  installed WebView2 runtime, read from the registry.
- **Not the driver failing to find the app.** Feeding it a deliberately wrong
  path produces a clear "no msedge binary at ..." error, so the path handling
  works.

The remaining suspects are in the automation layer: the browser arguments
msedgedriver passes to a WebView2 *host* application, which are meant for Edge
itself and can break environment creation.

**What this means practically:** the WebDriver harness is committed and correct
in shape, but it does not currently observe anything, so it must not be treated
as passing evidence. The verification that does hold is behavioural — launching
the real binary and checking effects only a mounted UI can produce.
