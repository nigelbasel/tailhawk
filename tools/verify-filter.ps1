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

    # §2.1 as resettled: Ctrl+L shows the docked filter panel with its add field focused; the
    # Ctrl+L opens the Filter dialog — a native #32770 — with focus in its Value box; typing
    # composes the expression live and Enter is OK, which adds the filter and runs the pass.
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FilterDlg {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public static IntPtr OfProcess(uint owner, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != owner) return true;
            var c = new StringBuilder(64); GetClassNameW(h, c, 64);
            var t = new StringBuilder(64); GetWindowTextW(h, t, 64);
            if (c.ToString() == "#32770" && t.ToString() == title) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
    $wsh.SendKeys('^l')
    $null = Wait-For { [FilterDlg]::OfProcess($proc.Id, 'Add Filter') -ne [IntPtr]::Zero } 'the Filter dialog' 10
    Write-Host 'the Filter dialog is a native #32770 window'
    Start-Sleep -Milliseconds 200
    $wsh.SendKeys('error')
    Start-Sleep -Milliseconds 300
    $wsh.SendKeys('{ENTER}')
    $null = Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match "$expected of $Lines" -and $proc.MainWindowTitle -notmatch 'scanning' } 'the pass to finish with the expected count'
    Start-Sleep -Milliseconds 600
    $proc.Refresh()
    $title = $proc.MainWindowTitle
    Write-Host "filtered: $title"

    $bmp = [Shot]::Client($hwnd)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    # Both themes' ERROR ink — the machine under test runs whichever theme its settings say, and
    # a harness that assumes dark reports zero ink on a screen visibly full of red.
    $errorDark = ConvertFrom-Rgbf 0.96 0.47 0.38
    $errorLight = ConvertFrom-Rgbf 0.72 0.12 0.08
    $number = ConvertFrom-Rgbf 0.61 0.79 0.94
    $errorPx = [Shot]::Count($bmp, $errorDark[0], $errorDark[1], $errorDark[2], 8) +
               [Shot]::Count($bmp, $errorLight[0], $errorLight[1], $errorLight[2], 8)
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
