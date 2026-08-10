<#
.SYNOPSIS
  Refuse to run a native executable Windows cannot prove Microsoft signed.

.DESCRIPTION
  `msedgedriver.exe` is downloaded over the network and then executed with the
  developer's privileges — by the setup script, and again by the E2E runner.
  Compromise of the download origin, the TLS path, or the publishing pipeline
  would otherwise be arbitrary code execution on the machine, and a bad copy
  written once persists through every later run because the version check
  executes the cached binary before deciding whether to replace it.

  Both the trust chain and the publisher are checked: a validly signed binary
  from someone else is still not the driver. The organisation is matched rather
  than a leaf thumbprint, because Microsoft's signing certificates rotate.

  Throws on any failure. Callers invoke it with `&` and let the terminating
  error propagate — `$LASTEXITCODE` belongs to the last *native* command and
  says nothing about a PowerShell script that threw.

.PARAMETER Path
  The executable to verify.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Path)

$ErrorActionPreference = 'Stop'

$resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).ProviderPath
$signature = Get-AuthenticodeSignature -LiteralPath $resolved
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "refusing unsigned or invalid executable: $resolved ($($signature.Status))"
}
if ($signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
    throw "refusing executable not signed by Microsoft Corporation: $resolved"
}
