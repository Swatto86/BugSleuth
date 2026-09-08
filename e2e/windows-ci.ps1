#Requires -Version 5.1
# WebView2 150+ ignores environment overrides for elevated hosts. Keep the
# machine-policy override restricted to this app on disposable CI runners.
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Only disposable GitHub runners may use this setup' }
$profile = Join-Path $env:RUNNER_TEMP ('bugsleuth-webview-' + [guid]::NewGuid().ToString())
$base = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\WebView2'
$values = @{
    AdditionalBrowserArguments = '--remote-debugging-port=0'
    UserDataFolder = $profile
}
$created = @()
try {
    foreach ($name in $values.Keys) {
        $key = Join-Path $base $name
        New-Item -Path $key -Force | Out-Null
        if ($null -ne (Get-Item $key).GetValue('bugsleuth-app.exe')) { throw 'Existing BugSleuth policy must not be overwritten' }
        New-ItemProperty -Path $key -Name 'bugsleuth-app.exe' -Value $values[$name] -PropertyType String | Out-Null
        $created += $key
    }
    & npm.cmd run --silent e2e:run
    $testExit = $LASTEXITCODE
} finally {
    foreach ($key in $created) { Remove-ItemProperty -Path $key -Name 'bugsleuth-app.exe' }
    if (Test-Path $profile) { Remove-Item -LiteralPath $profile -Recurse -Force }
}
exit $testExit
