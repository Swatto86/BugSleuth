<#
.SYNOPSIS
  Fail if the built frontend references a file it did not ship.

.DESCRIPTION
  A partial `ui/dist` is the worst kind of build failure: `vite build` succeeds,
  `cargo tauri build` succeeds, the installer is produced, the app launches, and
  the window is blank. Every check that looks at exit codes or at whether a
  window appeared says everything is fine.

  That is not hypothetical. A dist directory once shipped with the stylesheet
  present and the JavaScript missing — index.html referenced a bundle that was
  not there — and it was only caught by a WebDriver run finding no elements on
  the page.

  So: parse the built index.html and confirm every asset it names is on disk.
#>
[CmdletBinding()]
param(
    [string]$Dist = (Join-Path (Split-Path -Parent $PSScriptRoot) 'ui/dist')
)

$ErrorActionPreference = 'Stop'

$index = Join-Path $Dist 'index.html'
if (-not (Test-Path $index)) {
    Write-Host "no built frontend at $index - run: npm run build" -ForegroundColor Red
    exit 1
}

$html = Get-Content $index -Raw
$referenced = [regex]::Matches($html, '(?:src|href)="(/[^"]+)"') |
    ForEach-Object { $_.Groups[1].Value } |
    Sort-Object -Unique

if (-not $referenced) {
    Write-Host 'index.html references no bundled assets, which cannot be right' -ForegroundColor Red
    exit 1
}

$missing = @()
foreach ($asset in $referenced) {
    $path = Join-Path $Dist ($asset.TrimStart('/'))
    if (Test-Path $path) {
        $bytes = (Get-Item $path).Length
        if ($bytes -eq 0) { $missing += "$asset (empty)" }
    }
    else {
        $missing += $asset
    }
}

if ($missing.Count -gt 0) {
    Write-Host 'the built frontend is incomplete - the app would show a blank window:' -ForegroundColor Red
    $missing | ForEach-Object { Write-Host "  missing: $_" -ForegroundColor Red }
    Write-Host 'rebuild with: npm run build' -ForegroundColor Red
    exit 1
}

Write-Host "frontend assets OK ($($referenced.Count) referenced, all present)" -ForegroundColor Green
exit 0
