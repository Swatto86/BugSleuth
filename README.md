# BugSleuth

An adversarial, cross-vendor code review that produces a **ranked,
evidence-backed defect list you can act on without reading the code**.

It exists for a specific problem: shipping code you cannot personally review. An
AI wrote it, you cannot read it, and there is no independent reviewer. Asking one
model to review another model's output has correlated blind spots — especially
within the same family — so BugSleuth asks several different vendors, each with a
different mandate, mechanically verifies what they claim, and merges it into one
ranked list.

## What it is

**Findings are checked, not trusted.** A well-written hallucination and a real
bug look identical to someone who cannot read the code. So every finding is
checked mechanically: its quoted snippet must exist in the file it names, or it
is discarded. A model's claim that cannot be located in the code never reaches
the report.

**Diversity is manufactured, not hoped for.** Each review runs in a *lane* — a
narrow mandate with its own brief — because one generic "find bugs" prompt
collapses toward the same handful of findings whichever model you ask. The lanes
are correctness, security, contract, UX, and gate. Hover a lane heading in the
desktop app for the defects that lane is assigned to hunt.

Two front ends share one engine: a desktop app and the `bugsleuth` command.
Neither opens pull requests, talks to CI, or chats with the repository. Both
produce a defect list. The desktop app can also hand that list to a model that
edits the checkout, if you ask it to.

## Requirements

- **Git** on `PATH`. Cloning, Cursor and OpenCode reviews, and Apply all use it.
  The folder you review should be a git checkout.
- **At least one coding CLI** on `PATH`, already signed in with that CLI:
  - `claude` — `npm install -g @anthropic-ai/claude-code`, then run `claude` once
  - `codex` — install the Codex CLI and run `codex login`
  - `agent` — the Cursor Agent CLI; run `agent login`. In BugSleuth this vendor
    is `cursor`
  - `opencode` — install OpenCode, then `opencode auth login` or a local model
    in the global `opencode.json`
- A desktop webview runtime, listed under Install. The command-line binary does
  not need one.
- Releases are **Windows x64** and **Linux x64**. Apple Silicon can be built
  from source. Intel macOS is not a published target.

BugSleuth uses each CLI's own authentication. Billing follows the provider you
select. A local OpenCode model does not need a cloud subscription. API keys are
never accepted as arguments. Claude can use `ANTHROPIC_API_KEY` from the
environment when you pass `--use-api-key`; the other CLIs keep using their own
signed-in session.

## Install

Download the latest release from
[GitHub Releases](https://github.com/Swatto86/BugSleuth/releases/latest).

Every release ships a **directly runnable portable file** for each platform that
release built. The platform runtimes listed below are prerequisites. Use an
installer when you want a start-menu or package-manager entry.

| You want | Download |
|---|---|
| The desktop app, portable | `BugSleuth-portable-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |
| The desktop app, installed | `BugSleuth_x.y.z_x64-setup.exe`, `.msi`, `.deb`, `.dmg`, `.AppImage` |
| The command line only | `bugsleuth-cli-windows-x64.exe`, `-linux-x64`, `-macos-arm64` |

`x.y.z` is the version in the tag, without the leading `v`. Checksums sit beside
the files: `SHA256SUMS-windows-x64.exe.txt`, `SHA256SUMS-linux-x64.txt`, and
`SHA256SUMS-macos-arm64.txt` when that platform was built. On Linux or macOS,
download a checksum file into the same directory as the artifacts and run
`sha256sum -c SHA256SUMS-linux-x64.txt` (or the macOS file). On Windows,
`Get-FileHash -Algorithm SHA256` should match the hash on the same line as the
file name.

**A normal tag publishes Windows and Linux only.** Those are the files on
current releases, including v0.2.57. The macOS names in the table are what the
release workflow emits for Apple Silicon when it is run on the tag with
platforms set to `all`. They are not attached to an ordinary tag. Until a
release actually contains `BugSleuth-portable-macos-arm64`,
`bugsleuth-cli-macos-arm64`, and `BugSleuth_x.y.z_aarch64.dmg`, build those
from source.

### Windows

Install the [Evergreen WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
if it is not already present (it ships with current Edge).

- **Installer.** Run `BugSleuth_x.y.z_x64-setup.exe`. It installs for the
  current user, without administrator rights, to
  `%LOCALAPPDATA%\BugSleuth\bugsleuth-app.exe` and adds a Start menu entry.
  The `.msi` is the other Windows installer on the same release.
- **Portable.** Run `BugSleuth-portable-windows-x64.exe` from any folder. It is
  the same app without an installer.
- **Command line.** Download `bugsleuth-cli-windows-x64.exe`, put it on `PATH`,
  and call it as `bugsleuth` (renaming the file is fine). `bugsleuth --help`
  lists the commands.

### Linux

- **AppImage** (`BugSleuth_x.y.z_amd64.AppImage`). `chmod +x` the file and run
  it. It bundles its webview. Keep it at a stable path so the updater can
  replace that same file. If it exits immediately because FUSE is missing,
  install your distribution's FUSE 2 library, or start it with
  `--appimage-extract-and-run`.
- **Debian package** (`BugSleuth_x.y.z_amd64.deb`).
  `sudo apt install ./BugSleuth_x.y.z_amd64.deb`. The package name is
  `bugsleuth` and the command is `bugsleuth-app`. Apt installs the package's
  declared dependencies, including WebKitGTK 4.1.
- **Portable executable** (`BugSleuth-portable-linux-x64`). `chmod +x` and run
  it. This file is not a bundle: it needs
  [WebKitGTK 4.1 and the GTK/AppIndicator runtime libraries](https://v2.tauri.app/start/prerequisites/#linux)
  installed on the system.
- **Command line.** `chmod +x bugsleuth-cli-linux-x64`, put it on `PATH`, and
  call it as `bugsleuth`.

If an NVIDIA/Wayland launch exits with a protocol error or shows a blank window,
start the AppImage, package, or portable binary with
`WEBKIT_DISABLE_DMABUF_RENDERER=1`. In a desktop entry that is one line:
`Exec=env WEBKIT_DISABLE_DMABUF_RENDERER=1 /absolute/path/BugSleuth.AppImage`.

### macOS

No current release attaches macOS files. On Apple Silicon, build from source
(below). A maintainer build with platforms `all` publishes the portable binary,
the CLI, and `BugSleuth_x.y.z_aarch64.dmg`.

### Updates and removal

**Release builds of the desktop app check for a signed update** at startup and
every four hours. The file GitHub publishes for that check is the Windows NSIS
setup (`BugSleuth_x.y.z_x64-setup.exe`) or the Linux AppImage. On Windows the
setup installs into `%LOCALAPPDATA%\BugSleuth`. On Linux the AppImage is
replaced in place, so keep that file at a stable path. An update waits for
reviews, Apply, and other active work to finish, saves settings, then installs
and restarts. About → Check for updates does the same thing. The MSI, the
`.deb`, the portable binaries, and the CLI are separate downloads; replace
those by fetching the new release. Development builds never check.

Deleting the portable file, removing the package, or uninstalling from the
Start menu removes the app. Saved settings and run reports stay in the config
directory described below until you delete that folder.

## Configuration

The desktop app writes its own settings. You do not have to create the file.

| Platform | Settings and saved runs |
|---|---|
| Windows | `%APPDATA%\BugSleuth\settings.json` and `%APPDATA%\BugSleuth\runs\` |
| Linux and macOS | `$XDG_CONFIG_HOME/BugSleuth/` if that variable is set, otherwise `~/.config/BugSleuth/` |

Each repository's sweeps go in
`runs/<repo>-<16-hex-path-hash>`. The hash distinguishes checkouts that share
a folder name. A missing settings file means first launch and the Balanced
preset. A file that exists but is not valid JSON is left in place and reported;
the app does not overwrite it with defaults.

The command line reads a JSON file you pass to `--config`.
`bugsleuth.example.json` is a starting point: one Claude model, `sonnet`, on
every lane. Copy it and edit it.

Model ids are `vendor:model`. A bare name means Claude, so `sonnet` and
`claude:sonnet` are the same model. `codex:`, `cursor:`, and `opencode:` with
nothing after the colon use that CLI's own default. `cursor:` runs the `agent`
binary. `kilo:` and `kimi:` are refused; choose Claude, Codex, Cursor, or
OpenCode. A saved desktop row that still names a removed provider stays visible
so you can change it, and it will not run.

OpenCode model ids are read from `opencode models --pure --verbose`, including
configured Ollama, LM Studio, and custom provider entries. Select **opencode**
and enter the exact `provider/model` id, including any local tag (for example
`ollama/qwen3:8b`). In JSON and CLI arguments that is
`opencode:ollama/qwen3:8b`. Model boxes also accept an id that is not in the
catalogue yet. Configure OpenCode providers globally: a review checkout
discards repository-local agent and provider configuration. The selected
OpenCode model is checked before a multi-lane run, so an unavailable cloud
default does not block a working local model.

Effort is a separate field, not part of the model id. Leave it empty to keep
the CLI's default. **Agents**, on a Claude or Codex row, asks that CLI to split
the lane across parallel subagents and uses more tokens. Claude uses one small
Ultracode run with two foreground agents (at most 16 concurrently). Codex
chooses its own fan-out. Cursor and OpenCode cannot delegate, so the box is
unavailable for them.

Desktop presets replace the whole matrix:

| Preset | What it assigns |
|---|---|
| Cheap | `haiku` on every lane |
| Balanced | `sonnet` on every lane. This is the first-launch default |
| Deep | `opus` on every lane, plus `opencode:` on correctness and security |

Any lane with no model is reported as `NOT SWEPT` and the run is incomplete.
Two models on the same lane is deliberate: lanes buy coverage, a second model
buys depth. Drop a row whose CLI is not installed. A row that cannot run is
`NOT SWEPT` and the command exits non-zero, even when another model covered
that lane.

## First use

### Desktop app

A sweep takes tens of minutes. The app lives in the tray so you can close the
window and be told when it lands. Closing hides to the tray. **Quit** in the
window or the tray is the real exit.

1. Install and sign in to at least one coding CLI, in a terminal, before you
   open BugSleuth.
2. Start the app. The Providers panel lists each CLI that can be started, with
   its version. That is not the same as being signed in.
3. Press **Check sign-in**. It sends each installed CLI a one-word prompt.
   Hosted CLIs are allowed one minute. OpenCode is allowed five, because a
   local model's first call of a session loads its weights; later checks in
   that session are faster. This spends a trivial amount of quota.
4. Under **Repositories to Scan**, use **Add local folder…** or **Clone
   repositories…**. One folder path per line, up to 16. **Limit to paths** is
   optional and is guidance to the model, not a sandbox.
5. Leave **Balanced** selected, or tick lanes yourself. The matrix marks an
   empty column and says that lane will be `NOT SWEPT`. The footer states how
   many sweeps the run will pay for. **Run review** stays disabled until a
   repository is set and the banner says why.
6. Press **Run review**. The selected providers are checked before any lane
   starts. Progress streams into the result pane. Different vendors run
   together. Claude runs one session per Claude sweep across the repositories
   in flight — at most three repositories, and at most eight sessions. Every
   other vendor runs one sweep at a time, because its CLI shares one signed-in
   session on disk. **Stop** cancels the whole batch, including repositories
   still queued. Finished sweeps are kept.
7. When it ends, the result pane lists findings and a plain-text report split
   into coverage, summary, interpretation, limits, and actionable findings.
   **Copy report** copies that report. **Copy fix prompt** copies the work
   orders meant for a fixing agent, and the path under the button is a real
   `fix-prompt.md` in the run directory. A review that left a lane unswept
   says it is incomplete. It does not look like a clean bill of health.

**Clone repositories…** takes HTTPS or SSH addresses, one per line, a
destination parent, and an optional folder name for a single clone. It
downloads a full checkout of the default branch and adds each success to the
list. Private repositories use the Git credential helper or SSH agent you
already have. Signing in to a model provider does not authenticate Git. For
GitHub CLI users, `gh auth setup-git` connects an existing `gh` login to Git.
Do not put tokens or passwords in repository addresses. Existing destination
folders are never overwritten. Stopping a clone cancels it; incomplete
destinations are kept, so pick a new folder name to retry. Submodules are not
downloaded. A clone that runs longer than 30 minutes is stopped.

**Apply fixes** is separate from the review and stays off until you choose a
provider and model. It runs the same prompt as **Copy fix prompt**, with write
access, against a clean checkout, and reports what git observed. It refuses a
dirty tree. Fixes run one defect at a time and record progress beside the
report, so pressing Apply again after a stop continues. Up to three Codex
fixes can run in separate repositories; other vendors run one at a time.
**Push** and **tag a release** are extra checkboxes, off until you turn them
on, confirmed per apply, and not remembered as a convenience. Push sends only
the current branch to the upstream it already has, never with `--force`, and
refuses if any commit still credits a tool for the work.

**Reuse sweeps already completed** is on by default in the app, so pressing
Run again does not pay for sweeps already on disk. Untick it to review the
current tree from scratch. **Clear saved sweeps** deletes stored sweeps for
the repositories in the list. **Reset** deletes every saved run and leaves
settings in place. Reused sweeps describe the tree as it was when they were
taken.

Dark and light both follow the system by default. The title-bar theme control
switches them.

### Command line

```bash
bugsleuth preflight
```

Checks which provider CLIs can be started. It does not prove they are signed
in. Use **Check sign-in** in the desktop app for that.

```bash
bugsleuth sweep --repo <path> --lane correctness --model sonnet --json-out run.json
```

`--lane` is `correctness`, `security`, `contract`, `ux`, or `gate`. `--model`
takes `vendor:model`, as above. A single sweep prints a text report and, with
`--json-out`, writes the same report as JSON.

```bash
bugsleuth run --repo <path> --config bugsleuth.example.json --out-dir runs/ --resume
```

Runs every configured model × lane pair and merges the result. Each sweep is
written as it lands, so `--resume` continues a run that died without paying
again for sweeps already in `--out-dir`. Failed sweeps are retried. By default
one Claude model (`haiku`) re-grades the merged severities; pass
`--triage-model ""` to skip that pass. `--prompt-out <dir>` writes
`fix-prompt.md` plus one file per defect.

The per-sweep timeout is 45 minutes (2700 seconds) unless you pass
`--timeout-secs`. Claude also stops after 40 turns unless you pass
`--max-turns`. `--max-turns` and `--use-api-key` apply only to Claude. If a
Claude or Codex process times out, BugSleuth keeps its partial output and
resumes that session once for an answer-only pass. Recovered findings are
labelled as potentially incomplete. A run with no usable session id stays
`NOT SWEPT` rather than starting a second full review. `--claude-sessions`
lowers how many Claude sessions run at once (1–8). Other vendors stay serial.

```bash
bugsleuth judge run-a.json run-b.json run-c.json
```

Merges sweep files you already have into one ranked list, recording how many
vendors independently found each defect.

A coverage hole exits with status 2. A script can tell an unswept lane from a
clean report.

From a source checkout the same commands are
`cargo run -p bugsleuth-cli -- <command>`.

## Safety limits

These are why it can be pointed at a repository you care about.

- **A review cannot modify the code it reviews.** Claude runs with an explicit
  tool allowlist and no write tools. Codex runs with `--sandbox read-only`.
  Cursor and OpenCode use disposable git worktrees and read-only tool
  permissions.
- **The reviewed repository cannot alter its own review.** Every vendor runs
  with its customizations disabled, so a repository's own hooks, agent config,
  or rules are not loaded.
- **A lane that failed is never reported as clean.** It says `NOT SWEPT` with
  the reason, and the command exits non-zero. Silently omitting a lane that
  did not run is the most dangerous output this tool could produce.
- **Cross-lane severities are compared only after a complete triage pass.** By
  default, one model re-grades the merged list against one rubric. If triage
  is disabled, fails, or grades only part of the list, the report warns that
  the remaining grades are lane-relative and must not be compared across
  lanes.
- **API keys are read from the environment only**, never accepted as
  arguments, so they cannot reach a shell history or a process listing.
- **Publishing is opted into, narrow, and never forced.** Applying fixes can
  push what it committed, but only with the box ticked, only the branch you
  are on, only to the upstream that branch already has, and never with
  `--force`. A rejected push is reported and left alone.

Even a complete review has limits of method. No finding comes from running the
program, so races, leaks, and anything that depends on real data or a live
network were not looked for. Only code inside the repository was read; a
disagreement with a remote API or another service cannot be seen. Dependencies
were not checked against a vulnerability database. The UX lane reads code for
behavioural defects and cannot see what the app draws. A lane that reported
nothing means nothing was found, not that nothing is there.

Apply is the one path that writes. Nothing it writes has been checked by
anyone. The working tree must be clean before it starts, so the edits show up
in `git diff` and `git log`. Read them before you keep them.

## Troubleshooting

- **A provider pill says the CLI cannot be started.** Install it with the
  command in Requirements and confirm `claude`, `codex`, `agent`, or `opencode`
  runs in a terminal. A GUI launch does not load `PATH` from shell startup
  files, so a CLI that works only after `.bashrc` or a version manager may be
  invisible to the app. Put the binary on the default `PATH`, or start
  BugSleuth from a terminal that already has it.
- **Check sign-in fails.** The CLI's own message is the part that says what to
  do: run `claude`, `codex login`, `agent login`, or `opencode auth login`.
  OpenCode's first check of a session can take a few minutes while a local
  model loads. A listed model still needs a working provider and enough
  capacity to finish a review.
- **Run review stays disabled.** The banner under the matrix names the missing
  piece: no repository, a retired `kilo:` or `kimi:` row, or a vendor CLI that
  is not installed.
- **The window is blank, or Linux exits with a protocol error.** Install
  WebView2 on Windows, or WebKitGTK 4.1 for the unpackaged Linux binary and
  the `.deb`. For NVIDIA/Wayland, set `WEBKIT_DISABLE_DMABUF_RENDERER=1` as
  described under Install.
- **The AppImage will not start.** `chmod +x` it. If the error mentions FUSE,
  install FUSE 2 or pass `--appimage-extract-and-run`.
- **The Linux portable file will not start.** It needs the system WebKitGTK
  4.1 and AppIndicator libraries. The AppImage and the `.deb` are the copies
  that carry or install those dependencies.
- **There is no macOS download.** Current tags do not publish one. Build from
  source, or use a release that actually contains the `macos-arm64` files.
- **Settings did not load.** The footer names the file. Fix that JSON in
  place; replacing it with defaults would discard a recoverable config. Quit
  and start again after it parses.
- **A lane says NOT SWEPT.** The reason is in the report. Fix that cause and
  run again. With reuse left on, sweeps already on disk are kept.
- **The report describes old code.** Untick reuse, or clear saved sweeps,
  then run again.
- **Apply is refused.** The tree is dirty, the repository is not a git
  checkout, no fix prompt was saved, or another apply holds that repository.
  Commit or stash your own work first.
- **Updates never appear.** Development builds do not check. The updater
  installs the Windows setup executable or the Linux AppImage. A portable
  binary, MSI, `.deb`, or CLI stays at the version you downloaded until you
  replace the file. An AppImage is replaced only when it still lives at the
  path it was launched from.

## Build from source

You need Git, a C toolchain, and [Rust](https://rustup.rs/). The compiler
channel is pinned in `rust-toolchain.toml`; rustup selects it. The desktop
app also needs Node.js 22 and the webview packages for your OS. The command
line does not.

Linux desktop build dependencies, as the release workflow installs them:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev \
  patchelf libxdo-dev
```

Windows needs the [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
loader and the Visual Studio C++ build tools the `x86_64-pc-windows-msvc`
Rust target uses. Apple Silicon needs the Xcode command-line tools.

```bash
npm ci
npx tauri dev
```

`npx tauri build` writes `bugsleuth-app` (the portable app) to
`target/release/` and the installers under `target/release/bundle/`. Published
releases rename that portable binary to `BugSleuth-portable-<platform>`. The
Tauri CLI is the one pinned in `package.json`; `npm ci` installs it.

The command-line binary alone, with no webview packages:

```bash
cargo build --release -p bugsleuth-cli
```

That writes `target/release/bugsleuth` (`bugsleuth.exe` on Windows).

The full gate is `scripts/verify.sh`. On Windows,
`pwsh -File scripts/verify.ps1` runs that same script. It covers formatting,
clippy, tests, the frontend, and a debug build. Packaging is a separate step
after that gate: set `AGENT_RELEASE=1` and pass `--package` (PowerShell:
`-Package`).

End-to-end tests drive the real webview on Windows and Linux. macOS CI
compiles the app; WebDriver acceptance is not run there. Windows needs
`scripts/setup-e2e.ps1` so EdgeDriver matches the installed WebView2 version.
Linux needs `scripts/setup-tauri-driver.sh`, WebKitWebDriver, an X11 display
or Xvfb, and xclip.

```bash
npm run e2e
```

The suite is one short journey: boot, provider preflight, the uncovered-lane
warning, a real review, and the reviewed repository left untouched. Set
`BUGSLEUTH_E2E_MODEL` to pick the model; it defaults to `haiku`.
`BUGSLEUTH_E2E_LIVE=1` uses a real provider account. The routine gate uses
fixtures and spends no provider quota.

### Layout

| Crate | Responsibility |
|---|---|
| `bugsleuth-domain` | Lanes, findings, the JSON schemas. Types only — no I/O |
| `bugsleuth-provider` | One CLI adapter per vendor, plus shared subprocess handling |
| `bugsleuth-verify` | Anchor checking, git worktrees for isolated sweeps |
| `bugsleuth-judge` | Clustering, agreement counting, ranking |
| `bugsleuth-engine` | Briefs, planning, running, merging |
| `bugsleuth-cli` | The `bugsleuth` binary — argument parsing and printing |
| `src-tauri` | The desktop shell. A command deserializes, calls the engine, and serializes |

Dependencies point one way: everything may depend on `domain`, and `domain`
depends on nothing here. `ARCHITECTURE.md` is the longer map.
`RUNBOOK.md` is the acceptance journey to drive before shipping a build.
