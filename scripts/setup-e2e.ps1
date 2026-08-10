<#
.SYNOPSIS
  Fetch the WebDriver pieces the end-to-end suite needs.

.DESCRIPTION
  Two moving parts, and one of them is version-sensitive:

  - `tauri-driver`, which speaks WebDriver and launches the app.
  - `msedgedriver`, which drives the WebView2 webview inside it. This MUST match
    the machine's installed WebView2 runtime version. A mismatch fails with a
    session error that says nothing useful about the cause, so the version is
    read from the registry rather than guessed.

  Both land in `.webdriver/`, which is not tracked: the driver is a ~40 MB
  binary that is correct only for this machine.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root '.webdriver'
New-Item -ItemType Directory -Force $dest | Out-Null

Write-Host '== tauri-driver ==' -ForegroundColor Cyan
if (Get-Command tauri-driver -ErrorAction SilentlyContinue) {
    Write-Host '  already installed'
}
else {
    cargo install tauri-driver --locked
    if ($LASTEXITCODE -ne 0) { throw 'could not install tauri-driver' }
}

Write-Host '== WebView2 runtime ==' -ForegroundColor Cyan
$key = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$version = (Get-ItemProperty $key -ErrorAction SilentlyContinue).pv
if (-not $version) { throw 'the WebView2 runtime is not installed, so the app cannot render at all' }
Write-Host "  installed: $version"

$driver = Join-Path $dest 'msedgedriver.exe'
if (Test-Path $driver) {
    $have = (& $driver --version) -replace '.*WebDriver\s+([\d.]+).*', '$1'
    if ($have -eq $version) {
        Write-Host "  driver already matches ($have)"
        Write-Host "`nREADY" -ForegroundColor Green
        exit 0
    }
    Write-Host "  driver is $have but the runtime is $version - replacing" -ForegroundColor Yellow
}

Write-Host '== edge driver ==' -ForegroundColor Cyan
$zip = Join-Path $dest 'edgedriver.zip'
Invoke-WebRequest -Uri "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip" `
    -OutFile $zip -UseBasicParsing
Expand-Archive $zip -DestinationPath $dest -Force
# Before anything executes it. A modified archive served for the requested
# version would otherwise become arbitrary code execution here, on the line that
# reads the version back. The verifier throws; there is no $LASTEXITCODE to
# consult, because that belongs to the last native command and not to a .ps1.
& (Join-Path $PSScriptRoot 'assert-microsoft-signature.ps1') -Path $driver
Remove-Item $zip -Force
Write-Host "  installed: $(& $driver --version)"

Write-Host "`nREADY - run the suite with: npm run e2e" -ForegroundColor Green
