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
