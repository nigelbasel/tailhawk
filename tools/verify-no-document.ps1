# Proves the status bar says something **before there is a document** — the Welcome screen.
#
# `bar.set` used to sit inside `paint_inner`'s layout closure, past an early return taken when
# `pane_count == 0`. With no document there is no pane, so the branch returned first and the bar was
# never written: every first run showed a blank strip along the bottom, and any notice raised before
# a file opened — a failed open, a failed pipe, a refused remote source — had nowhere to appear.
#
# Nothing in the unit suite can see this. `status_text` composes the right words either way; the
# defect is *which call is reached*, so only a run reads the real control.
#
#   powershell tools/verify-no-document.ps1                  # the shipped binary
#   powershell tools/verify-no-document.ps1 -Exe path\to.exe # a saved one, to see the old behaviour
[CmdletBinding()]
param(
    [string]$Exe,
    [int]$Seconds = 20
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

if (-not $Exe) { $Exe = Get-TailhawkExe }
Write-Host "exe: $Exe"

# **No argument and no pipe**: the first-run surface. `stdin_kind().readable()` is false for an
# inherited console, so this takes neither the file path nor the pipe path — there is simply no
# document, which is the state the gate made unreportable.
$proc = Start-Process $Exe -PassThru

$failures = @()
try {
    $null = Wait-For { $proc.Refresh(); $proc.MainWindowHandle -ne 0 } 'the window to appear' $Seconds
    Start-Sleep -Milliseconds 1500
    $proc.Refresh()
    if ($proc.HasExited) { $failures += "the process exited with no document (code $($proc.ExitCode))" }

    $status = Get-StatusText $proc
    Write-Host "status bar with no document: '$status'"

    # What it says is `status_text`'s business and the unit suite's. What this run proves is that
    # the text reaches the control at all when no pane exists.
    if ([string]::IsNullOrWhiteSpace($status)) {
        $failures += 'the status bar is blank on the Welcome screen — the text never reached the control'
    }

    # **And that it is still docked after a resize.** `bar.resize()` forwards `WM_SIZE` so the
    # control retakes the bottom strip of the parent's *current* client rectangle. Its only call
    # site used to sit past the `pane_count == 0` return, so a window resized before the first file
    # was opened left the bar at its launch width — invisible while the bar was blank, and a
    # visible fault the moment it carried text. Measured rather than eyeballed: a screenshot would
    # not tell you whether the strip ends where the window does.
    Add-Type -Name BarGeom -Namespace Th -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
public delegate bool EnumProc(IntPtr h, IntPtr l);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int t, bool repaint);
public struct RECT { public int left, top, right, bottom; }
public static IntPtr FindBar(IntPtr parent) {
    IntPtr found = IntPtr.Zero;
    EnumChildWindows(parent, (h, l) => {
        var sb = new System.Text.StringBuilder(64);
        GetClassNameW(h, sb, sb.Capacity);
        if (sb.ToString() == "msctls_statusbar32") { found = h; return false; }
        return true;
    }, IntPtr.Zero);
    return found;
}
'@

    $main = $proc.MainWindowHandle
    $null = [Th.BarGeom]::MoveWindow($main, 120, 120, 1100, 700, $true)
    Start-Sleep -Milliseconds 1200

    $bar = [Th.BarGeom]::FindBar($main)
    if ($bar -eq [IntPtr]::Zero) {
        $failures += 'no msctls_statusbar32 child was found'
    } else {
        $br = New-Object Th.BarGeom+RECT
        $cr = New-Object Th.BarGeom+RECT
        $null = [Th.BarGeom]::GetWindowRect($bar, [ref]$br)
        $null = [Th.BarGeom]::GetClientRect($main, [ref]$cr)
        $barWidth = $br.right - $br.left
        $clientWidth = $cr.right - $cr.left
        Write-Host "after resize: bar is ${barWidth}px wide, client is ${clientWidth}px"
        # A status bar spans its parent's client width. A few pixels of slack covers the frame;
        # a bar left at its launch width is out by hundreds.
        if ([math]::Abs($barWidth - $clientWidth) -gt 8) {
            $failures += "the status bar did not re-dock after a resize: ${barWidth}px against a ${clientWidth}px client"
        }
    }

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
