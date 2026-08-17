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
#   pwsh tools/verify-find.ps1
#   pwsh tools/verify-find.ps1 -Query 'timeout|refused' -Lines 500000
[CmdletBinding()]
param(
    [string]$Query = 'ERROR',
    [int]$Lines = 200000,
    [string]$Log = "$env:TEMP\tailhawk-verify-find.log",
    [string]$Shot = "$env:TEMP\tailhawk-verify-find.png",
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\tailhawk.exe'
if (-not (Test-Path $exe)) { throw "build it first: cargo build --release   ($exe is missing)" }

Add-Type @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Shot {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }

    // The client area only. Title bar and border are the desktop's pixels, not ours, and counting
    // them would let a themed window colour decide whether the test passes.
    public static Bitmap Client(IntPtr hwnd) {
        RECT r; GetClientRect(hwnd, out r);
        POINT o = new POINT(); ClientToScreen(hwnd, ref o);
        Bitmap bmp = new Bitmap(r.R - r.L, r.B - r.T, PixelFormat.Format32bppArgb);
        using (Graphics g = Graphics.FromImage(bmp)) {
            g.CopyFromScreen(o.X, o.Y, 0, 0, bmp.Size, CopyPixelOperation.SourceCopy);
        }
        return bmp;
    }

    // Pixels within `tol` of one colour, per channel. A tolerance rather than an exact match
    // because the glyph pass blends ink over the background it is asked about.
    public static int Count(Bitmap bmp, int r, int g, int b, int tol) {
        Rectangle all = new Rectangle(0, 0, bmp.Width, bmp.Height);
        BitmapData d = bmp.LockBits(all, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        int n = 0;
        try {
            byte[] px = new byte[d.Stride * d.Height];
            Marshal.Copy(d.Scan0, px, 0, px.Length);
            for (int i = 0; i < px.Length; i += 4) {
                if (Math.Abs(px[i + 2] - r) <= tol && Math.Abs(px[i + 1] - g) <= tol && Math.Abs(px[i] - b) <= tol) { n++; }
            }
        } finally { bmp.UnlockBits(d); }
        return n;
    }
}
'@ -ReferencedAssemblies System.Drawing

# Without this the host is DPI-virtualised on a scaled display: GetClientRect and CopyFromScreen
# disagree about which pixels are the window's, and the capture lands on the desktop beside it.
[void][Shot]::SetProcessDPIAware()

function Wait-For([scriptblock]$Condition, [string]$What, [int]$Seconds = 30) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds 200
    }
    throw "timed out after ${Seconds}s waiting for: $What"
}

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

$proc = Start-Process $exe -ArgumentList $Log -PassThru
try {
    Wait-For { $proc.Refresh(); $proc.MainWindowHandle -ne 0 -and $proc.MainWindowTitle -match 'lines' } 'the window to open'
    $hwnd = $proc.MainWindowHandle
    Write-Host "opened: $($proc.MainWindowTitle)"

    $wsh = New-Object -ComObject WScript.Shell
    # AppActivate's return value says the request was made, not that it was honoured; the keys go
    # to whichever window is foreground when they are sent, so wait for that to be ours.
    # A bare Alt releases the foreground lock that otherwise keeps a busy user's window on top.
    Wait-For { $wsh.SendKeys('%'); $wsh.AppActivate($proc.Id) | Out-Null; [Shot]::GetForegroundWindow() -eq $hwnd } 'the window to take the foreground' 10
    Start-Sleep -Milliseconds 400

    $wsh.SendKeys('^f')
    Start-Sleep -Milliseconds 200
    # Braces, parens and the SendKeys metacharacters have to be escaped to arrive as themselves.
    $wsh.SendKeys(($Query -replace '([+^%~(){}\[\]])', '{$1}'))
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
    # The palette, as the render target writes it: the float channels times 255.
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
