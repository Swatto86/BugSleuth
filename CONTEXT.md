# BugSleuth context

BugSleuth is a Rust workspace with a Tauri 2 desktop shell and vanilla TypeScript
frontend. `ARCHITECTURE.md` describes the current boundaries; older
`AI_CONTEXT.md` entries are historical and may contradict current code.

Provider settings retain the compatible `vendor:model` format. Supported vendors
are Claude, Codex, Cursor and OpenCode. OpenCode model IDs are opaque,
including local tags, and catalogues are suggestions rather than an allowlist.
OpenCode providers must be configured globally because reviewed project
configuration is excluded from disposable review checkouts.

The release target for the current work is Windows and Omarchy, including local
OpenCode models. A release requires real webview acceptance, provider integration
evidence, installation through the active OS launcher and observed published
artifacts. Unit tests or a successful compile alone do not establish this.

The routine full gate uses debug builds and native WebDriver on Windows/Linux.
E2E runs in a disposable repository with isolated app settings and deterministic
CLI fixtures; `BUGSLEUTH_E2E_LIVE=1` enables separate real-provider acceptance.
Only processes owned by the harness may be terminated. Pinned driver archives
are checksum-verified before extraction; EdgeDriver must match WebView2.
Release packaging is a separate step after the debug gate. Tagged releases build Windows and Linux by default; manual releases can also
include macOS.

Kilo and Kimi adapters have been retired. Existing saved rows are preserved but
refused explicitly until the user selects a supported provider. Historical reports
remain readable.

Installed releases check signed updates at startup and every four hours. Updates
wait for repository operations to finish and settings to save before restarting.
Development builds never check for or install updates.

Repository cloning uses the installed Git CLI and existing credential helpers or
SSH agent. Clone creates a new folder beneath a chosen parent, selects it only
on success, and clears the previous path scope. Existing folders are refused;
failed/cancelled destinations are retained. HTTPS, SSH and absolute local sources
are supported; embedded HTTPS credentials are refused. Clones use the shared
operation lock and cancellation, with a 30-minute timeout. Submodules are not
initialized automatically. No credentials are stored by BugSleuth.

Desktop reviews accept up to 16 separate repository folders using one model
matrix and relative scope. Three repository reviews can be active; shared
provider slots serialize each vendor's sweeps and Claude triage across the
batch. Different vendors can work concurrently. Stop cancels the entire batch,
including repositories waiting to start. Each repository retains its own cache,
coverage, findings and fix prompt; the report selector binds Apply to that
report's repository. Each report remembers its own fixing provider/model and
effort. Explicit Apply actions may run concurrently in separate repositories;
provider slots serialize jobs using the same vendor. Results remain attached to
their repository. Stop all fixes cancels active and queued jobs. Duplicate,
nested and linked worktrees sharing Git metadata cannot apply together. Scans,
clearing, cloning and updates remain blocked until every apply finishes.
Publishing choices are session-only and confirmed separately for each apply.
Additional folders are saved compatibly alongside the existing primary folder.
Clear saved sweeps still targets only the primary folder.
