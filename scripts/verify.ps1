<#
.SYNOPSIS
  The full gate. Everything must pass before anything is called done.

.DESCRIPTION
  Cheapest checks first, so a formatting slip fails in seconds rather than after
  a release build. The frontend is included: a UI that type-checks and whose
  state rules are tested is the difference between gating the app and gating
  half of it.

  This does NOT run a packaged `tauri build` — that is minutes of LTO and
  belongs in the release runbook, not in the loop you run constantly. See
  `-Package` to include it.
#>
[CmdletBinding()]
param(
    # Also run the full packaged Tauri build. Slow; use before a release.
    [switch]$Package
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host '== rust fmt ==' -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt found unformatted files (run: cargo fmt --all)' }

    Write-Host '== clippy ==' -ForegroundColor Cyan
    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'clippy reported warnings' }

    Write-Host '== rust tests ==' -ForegroundColor Cyan
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }

    Write-Host '== frontend types ==' -ForegroundColor Cyan
    npm run --silent build
    if ($LASTEXITCODE -ne 0) { throw 'the frontend failed to type-check or build' }

    Write-Host '== frontend assets ==' -ForegroundColor Cyan
    & "$PSScriptRoot\check-frontend-assets.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'the built frontend is incomplete' }

    Write-Host '== frontend tests ==' -ForegroundColor Cyan
    npm test --silent
    if ($LASTEXITCODE -ne 0) { throw 'frontend tests failed' }

    Write-Host '== file sizes ==' -ForegroundColor Cyan
    & "$PSScriptRoot\check-file-size.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'a source file is over the hard line cap' }

    if ($Package) {
        # A full clean first, and this is not belt-and-braces. Tauri's own build
        # script decides dev-vs-production and cargo caches that decision. Once a
        # plain `cargo build --release` has recorded "dev", every later
        # `tauri build` reuses it and silently produces an app whose window is
        # blank without a dev server. `cargo clean -p bugsleuth-app` is not
        # enough - the poisoned cache belongs to the *dependency*.
        Write-Host '== packaged build (clean) ==' -ForegroundColor Cyan
        cargo clean --release
        cargo tauri build
        if ($LASTEXITCODE -ne 0) { throw 'the packaged Tauri build failed' }
    }
    else {
        # Everything EXCEPT the app crate. Building bugsleuth-app with plain
        # cargo is what poisons the cache described above, so the fast gate does
        # not do it; `-Package` is the way to build the app.
        Write-Host '== release build (libraries and CLI) ==' -ForegroundColor Cyan
        cargo build --release --workspace --exclude bugsleuth-app
        if ($LASTEXITCODE -ne 0) { throw 'release build failed' }
    }

    Write-Host "`nALL CHECKS PASSED" -ForegroundColor Green
}
finally {
    Pop-Location
}
