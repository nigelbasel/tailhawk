# Types a filter chip into the shipped binary and reads the filtered view off the screen -- E14's
# version of the check `verify-find.ps1` makes for search.
#
# The fixture has one ERROR line in every fifty. After `Ctrl+L`, `error`, `Enter`, the title must
# count exactly those, and the client area must show *only* rows carrying the ERROR colour -- which
# it can prove because every surviving row starts with the same word and every hidden row does not.
#
#   powershell tools/verify-filter.ps1
[CmdletBinding()]
param(
    [int]$Lines = 200000,
    [string]$Log = "$env:TEMP\tailhawk-verify-filter.log",
    [string]$Shot = "$env:TEMP\tailhawk-verify-filter.png",
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$every = 50
Write-Host "writing $Lines lines to $Log (an ERROR every $every)"
$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt $Lines; $i++) {
    if ($i % $every -eq 0) { $sw.WriteLine("2026-08-17 09:14:02.117 ERROR Api.Dispatch line $i failed to dispatch job $i") }
    else { $sw.WriteLine("2026-08-17 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms") }
}
$sw.Close()
$expected = [math]::Ceiling($Lines / $every)

$proc = Start-Tailhawk $Log
try {
    $hwnd = $proc.MainWindowHandle
    $wsh = New-Object -ComObject WScript.Shell

    $wsh.SendKeys('^l')
    Start-Sleep -Milliseconds 200
    $wsh.SendKeys('error')
    Start-Sleep -Milliseconds 200
    $proc.Refresh()
    Write-Host "typing:  $($proc.MainWindowTitle)"
    if ($proc.MainWindowTitle -notmatch '\+error') {
        throw "the chip did not reach the window: $($proc.MainWindowTitle)"
    }

    $wsh.SendKeys('{ENTER}')
    $null = Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match "$expected of $Lines" -and $proc.MainWindowTitle -notmatch 'scanning' } 'the pass to finish with the expected count'
    Start-Sleep -Milliseconds 600
    $proc.Refresh()
    $title = $proc.MainWindowTitle
    Write-Host "filtered: $title"

    $bmp = [Shot]::Client($hwnd)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $errorInk = ConvertFrom-Rgbf 0.96 0.47 0.38
    $number = ConvertFrom-Rgbf 0.61 0.79 0.94
    $errorPx = [Shot]::Count($bmp, $errorInk[0], $errorInk[1], $errorInk[2], 8)
    $numberPx = [Shot]::Count($bmp, $number[0], $number[1], $number[2], 8)
    $rowsOnScreen = [math]::Floor($bmp.Height / 27)
    $bmp.Dispose()

    Write-Host ''
    Write-Host "screenshot:      $Shot"
    Write-Host "error px:        $errorPx"
    Write-Host "number px:       $numberPx   (every INFO row carries '412' and '88ms'; an ERROR row carries only line and job numbers)"
    Write-Host "rows on screen:  ~$rowsOnScreen"

    $failures = @()
    # No literal middle dot in the pattern: Windows PowerShell reads a BOM-less script as ANSI.
    if ($title -notmatch "\+error .+? $expected of $Lines") { $failures += "the title should count $expected of $Lines" }
    # An ERROR word is ~120 px of ink per row; a screen of survivors is a screen of them.
    if ($errorPx -lt 60 * $rowsOnScreen) { $failures += "too little ERROR colour for a screen of survivors ($errorPx px)" }
    if ($proc.HasExited) { $failures += 'the process exited' }
    if ($failures) {
        $failures | ForEach-Object { Write-Host "FAIL: $_" -ForegroundColor Red }
        $failed = $true
    } else {
        Write-Host 'PASS' -ForegroundColor Green
    }
}
finally {
    # An `exit` inside the try would skip this block and leak the window; the exit code waits.
    if (-not $KeepWindow -and -not $proc.HasExited) { $proc.Kill() }
}
if ($failed) { exit 1 }
