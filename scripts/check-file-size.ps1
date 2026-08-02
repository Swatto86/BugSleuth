<#
.SYNOPSIS
  Fails if any source file exceeds the hard line cap.

.DESCRIPTION
  BugSleuth's owner does not read Rust, so "keep files small" cannot be enforced
  by code review — an 900-line file looks the same as a 200-line one in a diff
  summary. This turns the rule into something the build checks.

  Soft cap 300 lines (reported), hard cap 400 (fails). At the cap, split by
  responsibility, never by arbitrary halves.
#>
[CmdletBinding()]
param(
    [int]$SoftCap = 300,
    [int]$HardCap = 400
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$files = Get-ChildItem -Path $root -Recurse -File -Include '*.rs', '*.ts', '*.tsx' |
    Where-Object { $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\node_modules\\' }

$over = @()
$warn = @()
foreach ($file in $files) {
    # Every line, blank ones included. `Measure-Object -Line` skips empty
    # strings, so this silently undercounted by however many blank lines a file
    # had — three files sat 4, 12 and 27 lines over the hard cap while this
    # reported them as passing. Found the first time a second platform ran the
    # equivalent check and disagreed.
    $lines = @(Get-Content -LiteralPath $file.FullName).Count
    $relative = $file.FullName.Substring($root.Length + 1)
    if ($lines -gt $HardCap) {
        $over += [pscustomobject]@{ Path = $relative; Lines = $lines }
    }
    elseif ($lines -gt $SoftCap) {
        $warn += [pscustomobject]@{ Path = $relative; Lines = $lines }
    }
}

foreach ($item in $warn) {
    Write-Host "  over soft cap ($SoftCap): $($item.Path) — $($item.Lines) lines" -ForegroundColor Yellow
}

if ($over.Count -gt 0) {
    foreach ($item in $over) {
        Write-Host "  OVER HARD CAP ($HardCap): $($item.Path) — $($item.Lines) lines" -ForegroundColor Red
    }
    Write-Host "`nSplit these by responsibility before committing." -ForegroundColor Red
    exit 1
}

Write-Host "file sizes OK ($($files.Count) files, none over $HardCap lines)" -ForegroundColor Green
exit 0
