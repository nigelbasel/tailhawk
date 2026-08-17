# Presses Ctrl+O on the shipped binary, proves the dialog appears and the process survives it, then
# dismisses it. The first wiring aborted the process (a RefCell re-entered under the modal dialog),
# and only a run found it -- so this is the run.
#
#   powershell tools/verify-open.ps1
[CmdletBinding()]
param(
    [string]$Log = 'logs\agent.log',
    [string]$Shot = "$env:TEMP\tailhawk-verify-open.png"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$proc = Start-Tailhawk (Resolve-Path $Log).Path
$failures = @()
try {
    $main = $proc.MainWindowHandle
    $wsh = New-Object -ComObject WScript.Shell
    $wsh.SendKeys('^o')
    $null = Wait-For { -not $proc.HasExited -and [Shot]::GetForegroundWindow() -ne $main } 'the open dialog to take the foreground' 10
    Start-Sleep -Milliseconds 800
    $proc.Refresh()
    if ($proc.HasExited) { $failures += "the process exited under Ctrl+O (code $($proc.ExitCode))" }
    $fg = [Shot]::GetForegroundWindow()
    $bmp = [Shot]::Client($fg)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Host "dialog screenshot: $Shot ($($bmp.Width)x$($bmp.Height))"
    $bmp.Dispose()

    $wsh.SendKeys('{ESC}')
    Start-Sleep -Milliseconds 800
    $proc.Refresh()
    if ($proc.HasExited) { $failures += 'the process exited when the dialog was dismissed' }
    Write-Host "after Esc: $($proc.MainWindowTitle)"
    if ($proc.MainWindowTitle -notmatch 'lines') { $failures += 'the window did not come back after the dialog' }
    if ($failures) {
        $failures | ForEach-Object { Write-Host "FAIL: $_" -ForegroundColor Red }
        $failed = $true
    } else {
        Write-Host 'PASS' -ForegroundColor Green
    }
}
finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}
if ($failed) { exit 1 }
