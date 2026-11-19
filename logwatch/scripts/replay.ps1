param(
    [int]$DelayMs = 700,
    [switch]$Repeat
)
$ErrorActionPreference = 'Stop'
if ($DelayMs -lt 0) { throw 'DelayMs must be nonnegative.' }
$projectRoot = Split-Path -Parent $PSScriptRoot
$samplePath = Join-Path $projectRoot 'logs/application.log'
$demoPath = Join-Path $projectRoot 'logs/live-demo.log'
$sampleLines = [System.IO.File]::ReadAllLines($samplePath, [System.Text.Encoding]::UTF8)
$utf8 = [System.Text.UTF8Encoding]::new($false)
# Each append closes the file so viewers can reopen it on Windows.
[System.IO.File]::WriteAllText($demoPath, '', $utf8)
Write-Host "Replaying to $demoPath (Ctrl+C to stop)"
do {
    foreach ($line in $sampleLines) {
        if ($line -match '^\d{4}-\d{2}-\d{2} ') { Start-Sleep -Milliseconds $DelayMs }
        [System.IO.File]::AppendAllText($demoPath, $line + "`n", $utf8)
    }
} while ($Repeat)
