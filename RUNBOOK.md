# Release acceptance runbook

The journey to drive before shipping a change to the app, the provider adapters,
the run lifecycle, or packaging. Installing a published build is covered in
`README.md`. The routine suite uses subprocess fixtures and
spends no provider quota. Live mode performs real reviews and Apply, which can
take several minutes and uses the selected provider account.

The routine gate drives the real debug webview on Windows and Linux with an
isolated repository, app settings and subprocess fixtures. It covers review,
cancellation, clipboard, file-changing Apply, dirty-tree refusal, restart and
exit. Real provider acceptance runs separately with native authentication.

## Verify before packaging

```bash
npm ci
bash scripts/setup-tauri-driver.sh
# Windows also needs scripts/setup-e2e.ps1 for the matching EdgeDriver.
# Linux needs WebKitWebDriver, an X11 display (or Xvfb), and xclip.
bash scripts/verify.sh
```

The gate embeds the frontend with `tauri build --debug --no-bundle`. A plain
Cargo application build is insufficient because it can point at the dev server.
Caches do not need to be deleted.

For real-provider acceptance, run the same suite with a supported model:

```bash
BUGSLEUTH_E2E_LIVE=1 BUGSLEUTH_E2E_MODEL=haiku npm run e2e:run
```

This uses paid provider quota and edits only a disposable fixture repository.
After debug verification, package the release with `AGENT_RELEASE=1` set;
use an unsigned local configuration when the signing key is unavailable.

## Install it

On Windows, install the NSIS bundle you just built. It installs for the current
user to `%LOCALAPPDATA%\BugSleuth\bugsleuth-app.exe`:

```powershell
$version = (Get-Content .\src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
& ".\target\release\bundle\nsis\BugSleuth_${version}_x64-setup.exe" /S
```

On Linux, install the `.deb` under `target/release/bundle/deb/` or launch the
AppImage under `target/release/bundle/appimage/`. The file names use the same
version as `src-tauri/tauri.conf.json`. Test that copy, not the one in
`target/debug` or `target/release`. It is what people get.

## The journey

Start the installed app: `%LOCALAPPDATA%\BugSleuth\bugsleuth-app.exe` on
Windows, `bugsleuth-app` from the Debian package, or the AppImage on Linux.
Check each of these by looking, not by inference:

| # | Step | What must be true |
|---|---|---|
| 1 | App starts | Window appears; no blank page, no unstyled flash |
| 2 | Providers panel | Every configured CLI listed with a real version |
| 3 | Untick a lane's every box | The column head is marked and the banner names that lane as NOT SWEPT |
| 4 | Set a repository, one model, one lane; Untick **Re-grade every severity** | Footer shows one sweep and one round; Run enables; triage is off so this journey remains one pre-check and one sweep |
| 5 | **Run review** | The selected-provider pre-check finishes before lane progress streams into the result pane |
| 6 | Wait for it | Status reports the review is incomplete because only one lane was swept, and findings are listed |
| 7 | Check disk | The sweep file exists under the app config directory: `%APPDATA%\BugSleuth\runs\<repo>-<16-hex-path-hash>\<lane>-<model>.json` on Windows, or `$XDG_CONFIG_HOME/BugSleuth/runs/<repo>-<16-hex-path-hash>/` (`~/.config/BugSleuth/runs/` when `XDG_CONFIG_HOME` is unset) on Linux. `status.state` is `swept`, and `findings` is non-empty with real `file:line` anchors |
| 7b | Check a finding's `fix` | It has an approach, at least one edit naming a symbol, a verification command, and risks. An empty plan renders as "no fix plan" rather than as nothing |
| 7c | **Copy fix prompt** | The button appears when the run ends, copies, and briefly says "Copied". The path under it points at a real `fix-prompt.md` in the run directory |
| 7d | **Copy report** | The button appears when the run ends, copies the complete report, and briefly says "Copied" |
| 8 | Check the reviewed repo | Unchanged: `git status` clean, no `.bugsleuth-worktrees` left behind |
| 9 | Switch theme light and dark | Both readable; "Match system" follows the OS |
| 10 | Close the window | It hides; the process is still alive |
| 11 | Click the tray icon | The window returns with its state intact |
| 12 | Tray → Quit | The process exits, leaving no orphan |

Step 7c is the deliverable. Everything else in this list exists to produce a
prompt you can paste into a coding agent; if that button does not hand over
something usable, nothing above it counts.

Step 7 is the one that matters most for trusting the run. Findings shown in the window could in
principle come from anywhere; a sweep report on disk naming the model and
carrying anchors that resolve to real lines could not.

## Use a small repository with known defects

Point it at a small repository you already understand — the tool's own repo, or
any crate that builds in seconds. A sweep that returns nothing where you know a
defect exists means the pipeline ran and did nothing useful, which a status
check alone would not catch.

Model: `haiku` is enough. The journey is about the app, not the model.

A model that cannot hold the repository says so in the run, in those words.
Pick one the selected provider can actually run. Claude, Codex, Cursor, and
OpenCode are the supported providers; `kilo:` and `kimi:` are refused.

## Afterwards

```bash
pwsh -File scripts/verify.ps1 -Package
```

On Windows, confirm the portable executable is still self-contained — it must
import nothing outside `System32`:

```bash
dumpbin /dependents target/release/bugsleuth-app.exe
```

On Linux, the portable file `target/release/bugsleuth-app` still needs
WebKitGTK 4.1 and the AppIndicator libraries. The AppImage is the bundled
desktop file.
