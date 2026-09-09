# BugSleuth context

BugSleuth is a Rust workspace with a Tauri 2 desktop shell and vanilla TypeScript
frontend. `ARCHITECTURE.md` describes the current boundaries; older
`AI_CONTEXT.md` entries are historical and may contradict current code.

Provider settings retain the compatible `vendor:model` format. Supported vendors
are Claude, Codex, Cursor and OpenCode. OpenCode model IDs are opaque,
including local tags, and catalogues are suggestions rather than an allowlist.
Claude offers version-pinned IDs (including Fable 5 and 5.1) as well as latest
family aliases; saved aliases retain their meaning. The version list follows
Anthropic's documented model IDs because Claude Code has no non-interactive
catalogue command.
OpenCode providers must be configured globally because reviewed project
configuration is excluded from disposable review checkouts.

The release target for the current work is Windows and Omarchy, including local
OpenCode models. A release requires real webview acceptance, provider integration
evidence, installation through the active OS launcher and observed published
artifacts. Unit tests or a successful compile alone do not establish this.

The routine full gate uses debug builds and native WebDriver on Windows/Linux.
E2E runs in a disposable repository with isolated app settings and deterministic
CLI fixtures; `BUGSLEUTH_E2E_LIVE=1` enables separate real-provider acceptance.
`e2e/tauri.conf.json` gives debug acceptance its own bundle identifier while
keeping the production single-instance guard enabled, so an installed scan can
continue during verification. Build with `npm run e2e:build`.
Only processes owned by the harness may be terminated. Pinned driver archives
are checksum-verified before extraction; EdgeDriver must match WebView2.
Release packaging is a separate step after the debug gate. Tagged releases build Windows and Linux by default; manual releases can also
include macOS.

Kilo and Kimi adapters have been retired. Existing saved rows are preserved but
refused explicitly until the user selects a supported provider. Historical reports
remain readable.

Codex read-only runs explicitly enable the elevated Windows sandbox backend:
ignoring user config otherwise rejects even file reads with approval set to never.
Review responses must include `review_error`; blocked or missing completion status
becomes NOT SWEPT. Codex's wire schema is part of its cache contract, so older
Codex sweeps rerun while compatible Claude/Cursor caches remain reusable.
Live Astra review acceptance: `BUGSLEUTH_E2E_LIVE=1 npm run e2e:run -- --spec
e2e/specs/astra-review.spec.ts` after the debug build, with isolated test settings.

Installed releases check signed updates at startup and every four hours. Updates
wait for repository operations to finish and settings to save before restarting.
Development builds never check for or install updates.

Repository cloning uses the installed Git CLI and existing credential helpers or
SSH agent. A newline-separated list clones sequentially beneath one parent,
adding each successful checkout to Repositories to Scan and preserving scope.
An optional folder override applies only to single clones. Stop/failure retains
completed selections and leaves unattempted addresses in the dialog. Existing folders are refused;
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
up to three Codex fixes run simultaneously using ephemeral sessions and private
answer files; other vendors retain one slot each. A repository assignment board
exposes each saved report's provider, model, effort, start action and fix status.
Results remain attached to
their repository. Stop all fixes cancels active and queued jobs. Duplicate,
nested and linked worktrees sharing Git metadata cannot apply together. Scans,
clearing, cloning and updates remain blocked until every apply finishes.
Publishing choices are session-only and confirmed separately for each apply.
Live simultaneous Codex acceptance uses `BUGSLEUTH_E2E_LIVE_CODEX=1 npm run e2e:run
-- --spec e2e/specs/codex-live.spec.ts` after the debug build. It reviews two
disposable repositories with a deterministic scan fixture, overlaps real Codex fixes,
and runs an independent acceptance test outside both writable repositories.
The single Repositories to Scan list maps compatibly to the existing primary
folder and additional folders in settings. Clear saved sweeps explicitly targets
the first folder. Progress groups completed, reused and failed reviews by repository;
batch entries say reviewing/queued because provider slots are shared.
Each finished repository saves last-report.json atomically beside its sweep cache.
Startup restores reports without invoking providers; older runs reopen their saved
fix-prompt.md with a historical-coverage notice. Viewing a saved report does not
make its sweeps eligible for reuse or revalidate its findings against changed code.
