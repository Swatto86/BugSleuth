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

    Write-Host '== frontend tests ==' -ForegroundColor Cyan
    npm test --silent
    if ($LASTEXITCODE -ne 0) { throw 'frontend tests failed' }

    Write-Host '== file sizes ==' -ForegroundColor Cyan
    & "$PSScriptRoot\check-file-size.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'a source file is over the hard line cap' }

    if ($Package) {
        Write-Host '== packaged build ==' -ForegroundColor Cyan
        cargo tauri build
        if ($LASTEXITCODE -ne 0) { throw 'the packaged Tauri build failed' }
    }
    else {
        Write-Host '== release build ==' -ForegroundColor Cyan
        cargo build --release --workspace
        if ($LASTEXITCODE -ne 0) { throw 'release build failed' }
    }

    Write-Host "`nALL CHECKS PASSED" -ForegroundColor Green
}
finally {
    Pop-Location
}
