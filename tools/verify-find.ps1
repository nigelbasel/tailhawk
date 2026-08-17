# Drives the shipped binary through a search and reads the result off the screen.
#
# `SPEC.md` and this project's own history agree that a passing test is not the same as a working
# window: four of the defects that mattered most -- the black background, the grid of placeholder
# boxes, the stale set description, the selection the shell never handed to the painter -- were
# invisible to every unit test and obvious in one screenshot. This is the search feature's version
# of that check, and it is tracked in `tools/` rather than left in a scratch directory because the
# last harness that lived in one did not survive a reboot.
#
# It types into the real window with SendKeys rather than posting messages, because `Ctrl+F` is read
# with `GetKeyState` and a posted `WM_KEYDOWN` does not move the keyboard state.
#
#   powershell tools/verify-find.ps1
#   powershell tools/verify-find.ps1 -Query 'timeout|refused' -Lines 500000
[CmdletBinding()]
param(
    [string]$Query = 'ERROR',
    [int]$Lines = 200000,
    [string]$Log = "$env:TEMP\tailhawk-verify-find.log",
    [string]$Shot = "$env:TEMP\tailhawk-verify-find.png",
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

# A fixture with the matches in known places, so "did it find them" has an arithmetic answer.
$every = 50000
Write-Host "writing $Lines lines to $Log (a match every $every)"
$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt $Lines; $i++) {
    if ($i % $every -eq 0) { $sw.WriteLine("2026-08-16 09:14:02.117 ERROR Api.Dispatch line $i failed to dispatch job $i") }
    else { $sw.WriteLine("2026-08-16 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms") }
}
$sw.Close()
$expected = [math]::Ceiling($Lines / $every)

$proc = Start-Tailhawk $Log
try {
    $hwnd = $proc.MainWindowHandle
    $wsh = New-Object -ComObject WScript.Shell

    $wsh.SendKeys('^f')
    Start-Sleep -Milliseconds 200
    $wsh.SendKeys((ConvertTo-SendKeys $Query))
    Start-Sleep -Milliseconds 200
    $proc.Refresh()
    Write-Host "typing:  $($proc.MainWindowTitle)"
    if ($proc.MainWindowTitle -notmatch [regex]::Escape($Query)) {
        throw "the query did not reach the window: $($proc.MainWindowTitle)"
    }

    $wsh.SendKeys('{ENTER}')
    Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match 'of \d' -or $proc.MainWindowTitle -match 'no matches' } 'the search to report'
    Start-Sleep -Milliseconds 600
    $proc.Refresh()
    $title = $proc.MainWindowTitle
    Write-Host "found:   $title"

    $bmp = [Shot]::Client($hwnd)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $current = [Shot]::Count($bmp, 242, 158, 41, 12)   # CURRENT_MATCH_BG
    $other = [Shot]::Count($bmp, 92, 74, 15, 10)       # MATCH_BG
    $bmp.Dispose()

    Write-Host ''
    Write-Host "screenshot:        $Shot"
    Write-Host "current-match px:  $current"
    Write-Host "other-match px:    $other"
    Write-Host "matches expected:  $expected"

    $failures = @()
    if ($title -notmatch "of $expected\b") { $failures += "the title should say 'of $expected'" }
    if ($current -lt 100) { $failures += 'the current match is not painted in its own colour' }
    if ($proc.HasExited) { $failures += 'the process exited' }
    if ($failures) {
        $failures | ForEach-Object { Write-Host "FAIL: $_" -ForegroundColor Red }
        exit 1
    }
    Write-Host 'PASS' -ForegroundColor Green
}
finally {
    if (-not $KeepWindow -and -not $proc.HasExited) { $proc.Kill() }
}
