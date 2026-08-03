# BugSleuth

An adversarial, cross-vendor code review that produces a **ranked,
evidence-backed defect list you can act on without reading the code**.

It exists for a specific problem: shipping code you cannot personally review. An
AI wrote it, you cannot read it, and there is no independent reviewer. Asking one
model to review another model's output has correlated blind spots — especially
within the same family — so BugSleuth asks several different vendors, each with a
different mandate, and then makes them prove it.

See [PROGRESS.md](PROGRESS.md) for everything that has actually been measured,
including where it falls short. [NIGHT-REPORT.md](NIGHT-REPORT.md) is the first
night's record, kept as written; several of its numbers have been superseded.

## Install

Every release ships a **single file that runs with nothing installed** — no
runtime, no admin, no installer — for each platform. Take that one unless you
want the app in your start menu.

| You want | Download |
|---|---|
| The desktop app, portable | `BugSleuth-portable-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |
| The desktop app, installed | `BugSleuth_x.y.z_x64-setup.exe`, `.msi`, `.deb`, `.dmg`, `.AppImage` |
| The command line only | `bugsleuth-cli-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |

Checksums for each platform are published beside them as `SHA256SUMS-*.txt`.

**It updates itself.** BugSleuth checks GitHub for a newer release when you
press *Check for updates*, and installs it if you agree. Updates are signed; a
release that does not verify against the key built into the app is refused
before anything runs, so a tampered or unsigned download cannot install itself.
The check is a button rather than a background task because installing restarts
the app, and doing that during a review would throw away sweeps you have paid
for. Only the installed build updates itself — the portable `.exe` cannot
replace itself while running, so that one stays manual.

**Releases publish the Windows assets.** Linux and macOS are still supported and
still built from the same job — they are built on request rather than every
time, because building and verifying three sets of artifacts for a tool with one
user on Windows was work with nobody at the other end. To get them for a
release: Actions → **release** → Run workflow, choose that release's **tag**, and
set platforms to `all`. The assets appear on the existing release beside the
Windows ones.

**You also need at least one vendor CLI**, signed in, on your `PATH`: `claude`,
`codex` or `kilo`. BugSleuth drives them under your existing subscription — it
holds no API key and bills nothing itself. The Providers panel says which ones
it can start, though only a real sweep proves you are signed in.

## The two ideas

**Findings must carry their own proof.** A well-written hallucination and a real
bug look identical to someone who cannot read the code. So every finding is
checked mechanically: its quoted snippet must exist in the file it names, and
where possible a model is asked to write a test that *fails because of* the
defect. BugSleuth runs that test itself and only believes what it observes.

**Diversity is manufactured, not hoped for.** Each review runs in a *lane* — a
narrow mandate with its own brief — because one generic "find bugs" prompt
collapses toward the same handful of findings whichever model you ask.

## The desktop app

```bash
cargo tauri dev
```

```bash
cargo tauri build
```

It lives in the tray, because a sweep takes tens of minutes: start one, close
the window, and get told when it lands. Closing hides to the tray; the tray's
**Quit** is the only real exit.

The app's job that the command line cannot do is catching an uncovered lane
*before* you pay for the run. A lane with no model assigned still produces a
report — it just says NOT SWEPT — and that is easy to skim past. The lane matrix
marks the empty column and says so in as many words.

Dark and light both, following the system by default, switchable in the title
bar. Settings live in `%APPDATA%\BugSleuth\settings.json`, and each run's
per-sweep JSON goes in `%APPDATA%\BugSleuth\runs\<repo>`.

## Using the command line

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
cargo run -p bugsleuth-cli -- run --repo <path> --config bugsleuth.example.json --out-dir runs/ --resume
```

Runs every configured (model x lane) pair and merges the result. Models are
configured once and assigned the lanes they cover. Each sweep is written out as
it lands, so `--resume` picks up a run that died without paying for the sweeps
it already completed.

```bash
cargo run -p bugsleuth-cli -- run --repo <path> --config c.json --prove-top 5 --test-command "cargo test"
```

The same, then attempts to prove the top five merged defects with a failing
test. This is the expensive part — one model invocation and a full test run per
attempt — so it is off by default.

```bash
cargo run -p bugsleuth-cli -- judge run-a.json run-b.json run-c.json
```

Merges sweep files you already have into one ranked list of distinct defects,
recording how many vendors independently found each one.

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
- **"Not attempted" is never reported as "not proven".** Defects below the
  `--prove-top` cut are labelled unattempted, because "we did not try" and "we
  tried and failed" are different facts and the second is much stronger.
- **Severities are not compared across lanes.** A "high" from the security lane
  and a "high" from the correctness lane were assigned by models answering
  different questions, so a multi-lane report says so rather than implying a
  ranking nobody made.

## End-to-end tests

```bash
pwsh -File scripts/setup-e2e.ps1
```

```bash
npm run e2e
```

Drives the real release binary through its own webview. The Edge driver has to
match this machine's WebView2 runtime version exactly — the setup script reads
that from the registry rather than guessing, because a mismatch fails with an
error that says nothing about the cause.

The suite is deliberately one short journey, not a spec per feature: boot,
provider preflight, the uncovered-lane warning, **a real review with a real
model**, and the reviewed repository left untouched. It asserts effects rather
than calls — findings in the window could come from anywhere, but a sweep report
landing in the app's runs directory naming the model that produced it could not.

Set `BUGSLEUTH_E2E_MODEL` to pick the model; it defaults to `haiku` because the
journey should cost as little as a real run can.

## Building

```bash
pwsh -File scripts/verify.ps1
```

Rust formatting, clippy with warnings as errors, the Rust tests, the frontend
type-check and its tests, a check that no source file exceeds 400 lines, and a
release build. Add `-Package` to include the full packaged Tauri build, which is
minutes of link-time optimisation and belongs before a release rather than in
the loop.

## Layout

| Crate | Responsibility |
|---|---|
| `bugsleuth-domain` | Lanes, findings, proof verdicts. Types only — no I/O, depends on nothing else here |
| `bugsleuth-provider` | One CLI adapter per vendor, plus shared subprocess handling |
| `bugsleuth-verify` | Anchor checking, git worktrees, test execution |
| `bugsleuth-judge` | Clustering, agreement counting, ranking |
| `bugsleuth-engine` | The crate that composes the others: briefs, planning, running, merging, proving |
| `bugsleuth-cli` | The `bugsleuth` binary — argument parsing and printing |
| `src-tauri` | The desktop shell. Commands are deserialize, call the engine, serialize |

Dependencies point one way: everything may depend on `domain`, and `domain`
depends on nothing. `judge` does not know `provider` exists. Both front ends run
the same engine rather than two implementations of it — the alternative is
exactly the kind of quiet divergence this tool exists to catch elsewhere.
