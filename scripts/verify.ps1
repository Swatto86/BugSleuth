<#
.SYNOPSIS
  The full gate. Everything must pass before anything is called done.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host '== fmt ==' -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt found unformatted files (run: cargo fmt --all)' }

    Write-Host '== clippy ==' -ForegroundColor Cyan
    cargo clippy --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'clippy reported warnings' }

    Write-Host '== tests ==' -ForegroundColor Cyan
    cargo test --all
    if ($LASTEXITCODE -ne 0) { throw 'tests failed' }

    Write-Host '== build ==' -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'release build failed' }

    Write-Host '== file sizes ==' -ForegroundColor Cyan
    & "$PSScriptRoot\check-file-size.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'a source file is over the hard line cap' }

    Write-Host "`nALL CHECKS PASSED" -ForegroundColor Green
}
finally {
    Pop-Location
}
