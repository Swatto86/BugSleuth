# BugSleuth

An adversarial, cross-vendor code review that produces a **ranked,
evidence-backed defect list you can act on without reading the code**.

It exists for a specific problem: shipping code you cannot personally review. An
AI wrote it, you cannot read it, and there is no independent reviewer. Asking one
model to review another model's output has correlated blind spots — especially
within the same family — so BugSleuth asks several different vendors, each with a
different mandate, mechanically verifies what they claim, and merges it into one
ranked list.

## Install

Every release ships a **directly runnable portable file** for each supported
platform. The platform runtimes listed below are prerequisites. Use the installer
if you want the app added to your start menu.

| You want | Download |
|---|---|
| The desktop app, portable | `BugSleuth-portable-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |
| The desktop app, installed | `BugSleuth_x.y.z_x64-setup.exe`, `.msi`, `.deb`, `.dmg`, `.AppImage` |
| The command line only | `bugsleuth-cli-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |

Checksums for each platform are published beside them as `SHA256SUMS-*.txt`.

**It updates itself.** BugSleuth checks signed updates at startup and every four
hours. It waits for reviews, Apply and other active work to finish, saves settings,
then installs and restarts automatically. About → Check for updates triggers the
same process. Development builds never update themselves. Keep the Linux AppImage
at a stable path so its updater can replace it in place.

**Tagged releases publish Windows and Linux assets.** On Omarchy, use the
Linux AppImage. Windows needs the [Evergreen WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/); the unpackaged Linux executable needs
[WebKitGTK 4.1 and the GTK/AppIndicator runtime libraries](https://v2.tauri.app/start/prerequisites/#linux). macOS artifacts can be
added by running the release workflow against the release tag with platforms
set to `all`.

On Omarchy, keep the AppImage at `~/.local/share/bugsleuth/BugSleuth.AppImage`,
make it executable, and point your desktop launcher at that path. Replace that
file to upgrade manually; keep the previous file separately if you need rollback.
Remove the file and its desktop launcher to uninstall; saved settings are retained.
If an NVIDIA/Wayland launch exits with a protocol error or shows a blank window,
launch with `WEBKIT_DISABLE_DMABUF_RENDERER=1` (use `Exec=env
WEBKIT_DISABLE_DMABUF_RENDERER=1 /absolute/path/BugSleuth.AppImage` on one line in
the desktop entry). This was required on the Omarchy acceptance machine.

**You also need at least one configured coding CLI** on your `PATH`: `claude`,
`codex`, `agent` (Cursor), or `opencode`. BugSleuth uses each
CLI's own authentication and provider configuration. Billing follows the selected
provider; local OpenCode models do not require a cloud subscription.

OpenCode models are read from `opencode models --pure --verbose`, including
configured Ollama, LM Studio and custom provider entries. Select **opencode**
and enter the exact `provider/model` ID, including any local tag (for example
`ollama/qwen3:8b`). In JSON settings and CLI arguments this becomes
`opencode:ollama/qwen3:8b`. Model boxes also accept IDs absent from a catalogue,
so a newly available model need not wait for a BugSleuth release. OpenCode
reasoning variants are read from the selected model's catalogue entry.

Configure OpenCode providers globally: review checkouts discard repository-local
agent and provider configuration. Reviews use a private agent allowing only
read, glob and grep; Apply permits edits and shell commands in a separate
invocation. The selected OpenCode model is checked before a multi-lane run,
so an unavailable cloud default does not prevent use of a working local model.

Before the desktop app starts a lane, it asks the selected providers for a short
answer; the Providers panel can also check the configured defaults. A listed model
still needs a working provider connection and sufficient capacity to complete a review.

## The two ideas

**Findings are checked, not trusted.** A well-written hallucination and a real
bug look identical to someone who cannot read the code. So every finding is
checked mechanically: its quoted snippet must exist in the file it names, or it
is discarded. A model's claim that cannot be located in the code never reaches
the report.

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
Hover a lane heading for the defects that lane is assigned to hunt.
The optional **Agents** box asks Claude or Codex to split that row's lane work
across parallel subagents, which uses more tokens. Claude uses one small
Ultracode run with two foreground agents (the runtime allows at most 16 concurrently);
Codex chooses its own fan-out. It is unavailable for Cursor and OpenCode because their review modes cannot delegate.

Finished results show expandable finding cards and a plain-text report split
into coverage, summary, interpretation, limits, and actionable findings. **Copy
report** copies that complete report; **Copy fix prompt** copies the detailed
work orders intended for a fixing agent.

Dark and light both, following the system by default, switchable in the title
bar. Settings live in `%APPDATA%\BugSleuth\settings.json`, and each run's
per-sweep JSON goes in
`%APPDATA%\BugSleuth\runs\<repo>-<16-hex-path-hash>`. The hash distinguishes
checkouts that share the same folder name.

## Using the command line

```bash
cargo run -p bugsleuth-cli -- preflight
```

Checks which provider CLIs can be started. It does **not** prove they are signed
in; use **Check sign-in** in the desktop app for that.

```bash
cargo run -p bugsleuth-cli -- sweep --repo <path> --lane correctness --model sonnet --json-out run.json
```

`--model` takes `vendor:model`. A bare name means Claude. `codex:`, `cursor:` and `opencode:`
with nothing after them use each CLI's own default.

```bash
cargo run -p bugsleuth-cli -- run --repo <path> --config bugsleuth.example.json --out-dir runs/ --resume
```

Runs every configured (model x lane) pair and merges the result. Models are
configured once and assigned the lanes they cover. Different providers run
together, but one provider's sweeps run sequentially because the CLIs publish no
safe process limit. Each sweep is written out as it lands, so `--resume` picks
up a run that died without paying for the sweeps it already completed.

If an individual Claude or Codex process times out, BugSleuth keeps its
partial CLI output and resumes that same session once for an answer-only pass.
It also handles each provider's native interruption signal: Claude continues an
interrupted resumed turn and Codex resumes a transient failed turn. Recovered findings are labelled as
potentially incomplete; a run with no usable session id remains `NOT SWEPT`
rather than silently restarting from scratch.

```bash
cargo run -p bugsleuth-cli -- judge run-a.json run-b.json run-c.json
```

Merges sweep files you already have into one ranked list of distinct defects,
recording how many vendors independently found each one.

## What it will not do

It does not open pull requests, integrate with CI, or chat with your codebase.
It produces a defect list. That is the whole scope, deliberately.

It will hand that list to a model that edits your code, if you ask it to — the
app's Apply panel, which runs the same prompt the Copy button gives you against
a clean checkout and reports what git observed rather than what the model
claimed. Optionally it will then push those commits to the branch's existing
upstream. Both are off until you turn them on, and neither is part of a review.

## Safety properties

These are the ones worth knowing, because they are why it can be pointed at a
repository you care about:

- **A review cannot modify the code it reviews.** Claude runs with an explicit
  tool allowlist and no write tools. Codex runs with `--sandbox read-only`.
  Cursor and OpenCode use disposable git worktrees and read-only tool permissions.
- **The reviewed repository cannot alter its own review.** Every vendor runs with
  its customizations disabled, so a repository's own hooks, agent config or rules
  are not loaded.
- **A lane that failed is never reported as clean.** It says `NOT SWEPT` with the
  reason and exits non-zero. Silently omitting a lane that did not run is the
  most dangerous output this tool could produce.
- **API keys are read from the environment only**, never accepted as arguments,
  so they cannot reach a shell history or a process listing.
- **Publishing is opted into, narrow, and never forced.** Applying fixes can push
  what it committed, but only with the box ticked, only the branch you are on,
  only to the upstream that branch already has, and never with `--force`. A
  rejected push is reported and left alone. It refuses outright if any commit
  still credits a tool for the work — that is the one thing pushing makes
  permanent.
- **Cross-lane severities are compared only after a complete triage pass.** By
  default, one model re-grades the merged list against one rubric. If triage is
  disabled, fails, or grades only part of the list, the report warns that the
  remaining grades are lane-relative and must not be compared across lanes.

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
provider preflight, the uncovered-lane warning, selected-provider pre-check,
**a real review with a real model**, and the reviewed repository left untouched.
It asserts effects rather than calls — findings in the window could come from
anywhere, but a sweep report landing in the app's runs directory naming the
model that produced it could not.

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
| `bugsleuth-domain` | Lanes, findings, the JSON schemas. Types only — no I/O, depends on nothing else here |
| `bugsleuth-provider` | One CLI adapter per vendor, plus shared subprocess handling |
| `bugsleuth-verify` | Anchor checking, git worktrees for isolated sweeps |
| `bugsleuth-judge` | Clustering, agreement counting, ranking |
| `bugsleuth-engine` | The crate that composes the others: briefs, planning, running, merging |
| `bugsleuth-cli` | The `bugsleuth` binary — argument parsing and printing |
| `src-tauri` | The desktop shell. Commands are deserialize, call the engine, serialize |

Dependencies point one way: everything may depend on `domain`, and `domain`
depends on nothing. `judge` does not know `provider` exists. Both front ends run
the same engine rather than two implementations of it — the alternative is
exactly the kind of quiet divergence this tool exists to catch elsewhere.

### Clone a repository

Choose **Clone…** beside the repository folder, enter its HTTPS or SSH address,
choose a destination parent and a new folder name, then **Clone and select**.
BugSleuth downloads a full default-branch checkout and selects it for review.
Git must be installed. Private repositories use your existing Git credential
helper or SSH agent; signing into a model provider does not authenticate Git.
For GitHub CLI users, `gh auth setup-git` connects an existing GitHub login to
Git's credential helper. Do not put tokens or passwords in repository addresses.

Existing destination folders are never overwritten. Stop cloning cancels the
operation; incomplete destinations are kept for inspection, so use a new folder
name when retrying. Submodules are not downloaded automatically.

The desktop clone acceptance uses a real local Git repository. To additionally
check private-repository access with your existing credentials, set
`BUGSLEUTH_E2E_CLONE_URL` to an accessible, small private repository with committed
files before running `npm run e2e`. The clone stays in the disposable E2E workspace.
