<#
.SYNOPSIS
  The full gate. Everything must pass before anything is called done.

.DESCRIPTION
  **One definition, not two.** This used to be a complete PowerShell gate
  running alongside `verify.sh`, and keeping the pair in step by hand failed
  three times in a single day:

    - They counted lines differently. `Measure-Object -Line` skips blank lines,
      so this gate undercounted every file and passed three that were over the
      hard cap — and files were split against those wrong numbers.
    - They checked different file types, so the first cross-platform CI run
      failed on a file this side had never looked at.
    - They built different things, so the release collected a CLI binary that
      existed only because this gate happened to produce it. v0.2.0 published
      on Windows and failed everywhere else.

  Every one of those was caught by the *other* gate rather than by review,
  which is the argument for not having two of them. Mirroring by inspection
  does not work; the only reliable parity is having nothing to mirror.

  So this is a shim now. `verify.sh` is the gate, run here through the Bash that
  ships with Git for Windows — the same shell the CI matrix already uses on this
  platform. The parameter is kept so hooks, the runbook and muscle memory keep
  working.
#>
[CmdletBinding()]
param(
    # Also run the full packaged Tauri build. Slow; use before a release.
    [switch]$Package
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Git's own bash, not whatever `bash` PATH happens to offer.
#
# On a machine with WSL enabled, `bash` on PATH is the WSL launcher in
# WindowsApps. Handed this gate it answers "Windows Subsystem for Linux has no
# installed distributions" and exits 1, so the gate looks broken when nothing
# is wrong with it — and the failure says nothing about the shell being the
# problem. Git Bash is also the shell the pre-push hook and the CI matrix
# already use on this platform, which is the point: one definition of green,
# reached the same way everywhere.
function Find-GitBash {
    $git = (Get-Command git -ErrorAction SilentlyContinue).Source
    if ($git) {
        # <git>\cmd\git.exe and <git>\bin\git.exe both put bash at <git>\bin.
        $candidate = Join-Path (Split-Path -Parent (Split-Path -Parent $git)) 'bin\bash.exe'
        if (Test-Path $candidate) { return $candidate }
    }
    foreach ($found in @(Get-Command bash -All -ErrorAction SilentlyContinue)) {
        if ($found.Source -notmatch '\\WindowsApps\\') { return $found.Source }
    }
    return $null
}

$bash = Find-GitBash
if (-not $bash) {
    throw 'Git Bash was not found. It ships with Git for Windows; install Git or put its bin\bash.exe on PATH.'
}

$gate = (Join-Path $PSScriptRoot 'verify.sh') -replace '\\', '/'
$gateArgs = @($gate)
if ($Package) { $gateArgs += '--package' }

Push-Location $root
try {
    & $bash @gateArgs
    if ($LASTEXITCODE -ne 0) { throw "the gate failed (exit $LASTEXITCODE)" }
}
finally {
    Pop-Location
}
