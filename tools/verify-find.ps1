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

    # §2.1 as resettled: Ctrl+F opens the classic modeless Find dialog. The query is typed into
    # the dialog's own edit control, and Enter is Find Next, its default button.
    #
    # **Scoped to the process, not the desktop.** A bare FindWindowW('#32770','Find') matches any
    # application's Find dialog — the first draft failed against Tailhawk while faithfully
    # reporting on a dialog belonging to something else entirely.
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FindDlg {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public static IntPtr OfProcess(uint owner) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != owner) return true;
            var c = new StringBuilder(64); GetClassNameW(h, c, 64);
            var t = new StringBuilder(64); GetWindowTextW(h, t, 64);
            if (c.ToString() == "#32770" && t.ToString() == "Find") { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
    $wsh.SendKeys('^f')
    $null = Wait-For { [FindDlg]::OfProcess($proc.Id) -ne [IntPtr]::Zero } 'the Find dialog' 10
    Write-Host 'the Find dialog is a native #32770 window owned by the app'
    Start-Sleep -Milliseconds 200
    $wsh.SendKeys((ConvertTo-SendKeys $Query))
    Start-Sleep -Milliseconds 200

    $wsh.SendKeys('{ENTER}')
    Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match 'of \d' -or $proc.MainWindowTitle -match 'no matches' } 'the search to report'
    Start-Sleep -Milliseconds 600
    # Esc closes the dialog before the screenshot, so the grid is what is photographed — and
    # proves the dialog dismisses the way the standard one does.
    $wsh.SendKeys('{ESC}')
    Start-Sleep -Milliseconds 400
    if ([FindDlg]::OfProcess($proc.Id) -ne [IntPtr]::Zero) {
        throw 'Esc did not close the Find dialog'
    }
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

    # One of the four options exercised end to end: Whole word turns a partial token into no
    # matches — ERRO is inside every ERROR, and the boundary is what keeps it from counting.
    $wsh.SendKeys('^f')
    $null = Wait-For { [FindDlg]::OfProcess($proc.Id) -ne [IntPtr]::Zero } 'the Find dialog again' 10
    Start-Sleep -Milliseconds 250
    $wsh.SendKeys('ERRO')
    $wsh.SendKeys('%w')
    Start-Sleep -Milliseconds 150
    $wsh.SendKeys('{ENTER}')
    $null = Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match 'no matches' } 'whole word to exclude the partial token' 10
    Write-Host 'whole word: the partial token finds nothing, as it should'
    $wsh.SendKeys('{ESC}')
    Start-Sleep -Milliseconds 300

    $failures = @()
    if ($title -notmatch "of $expected\b") { $failures += "the title should say 'of $expected'" }
    if ($current -lt 100) { $failures += 'the current match is not painted in its own colour' }
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
