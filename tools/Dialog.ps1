# Finds and closes the application's own dialog windows.
#
# **Every lookup is scoped to a process id, and that is the whole point of the file.** `FindWindowW`
# searches the entire desktop, so a harness that asks for a `#32770` titled `Find` will happily be
# answered by some other application's Find dialog — `verify-find.ps1` failed exactly that way once,
# and reported the failure faithfully while testing nothing. Enumerating the windows of a known pid
# is the version that cannot lie.
#
# Dot-source after `Screen.ps1`:
#
#     . (Join-Path $PSScriptRoot 'Screen.ps1')
#     . (Join-Path $PSScriptRoot 'Dialog.ps1')

if (-not ('Dlg' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class Dlg {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    public const uint WM_CLOSE = 0x0010;

    // Every visible top-level window of one process, as "class|title" - the dialogs and the main
    // window together, because what a caller usually wants to know is which of them appeared.
    public static string[] Windows(uint owner) {
        List<string> found = new List<string>();
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != owner || !IsWindowVisible(h)) return true;
            StringBuilder c = new StringBuilder(64); GetClassNameW(h, c, 64);
            StringBuilder t = new StringBuilder(256); GetWindowTextW(h, t, 256);
            found.Add(c.ToString() + "|" + t.ToString());
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }

    // The first visible window of `owner` whose class and title match. An empty title matches any.
    public static IntPtr Find(uint owner, string cls, string title) {
        IntPtr hit = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != owner || !IsWindowVisible(h)) return true;
            StringBuilder c = new StringBuilder(64); GetClassNameW(h, c, 64);
            if (c.ToString() != cls) return true;
            StringBuilder t = new StringBuilder(256); GetWindowTextW(h, t, 256);
            if (title.Length > 0 && t.ToString() != title) return true;
            hit = h; return false;
        }, IntPtr.Zero);
        return hit;
    }
}
'@
}

# The dialog class. Every dialog Tailhawk raises is one of these, whether it comes from the
# application's own template, from `ChooseFontW`, or from `TaskDialogIndirect`.
$script:DIALOG_CLASS = '#32770'

function Get-Dialog([int]$OwnerPid, [string]$Title) {
    [Dlg]::Find([uint32]$OwnerPid, $script:DIALOG_CLASS, $Title)
}

function Wait-Dialog([int]$OwnerPid, [string]$Title, [int]$Seconds = 10) {
    $null = Wait-For { (Get-Dialog $OwnerPid $Title) -ne [IntPtr]::Zero } "the '$Title' dialog" $Seconds
    Get-Dialog $OwnerPid $Title
}

# Closes a dialog and waits for it to go. `WM_CLOSE` rather than a keystroke, because it reaches
# the dialog wherever the foreground happens to be, and every dialog here reads it as Cancel.
function Close-Dialog([int]$OwnerPid, [string]$Title, [int]$Seconds = 8) {
    $dlg = Get-Dialog $OwnerPid $Title
    if ($dlg -eq [IntPtr]::Zero) { return $true }
    [void][Dlg]::PostMessageW($dlg, [Dlg]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    try {
        $null = Wait-For { (Get-Dialog $OwnerPid $Title) -eq [IntPtr]::Zero } "the '$Title' dialog to close" $Seconds
        $true
    } catch { $false }
}

# A dialog's own screen rect. `shot-window.ps1` pins the *main* window topmost, which puts it in
# front of the very dialog a caller wants photographed; capturing by the dialog's own rect is the
# way round that.
function Get-DialogRect([IntPtr]$Dialog) {
    $r = New-Object Dlg+RECT
    if (-not [Dlg]::GetWindowRect($Dialog, [ref]$r)) { throw 'GetWindowRect failed' }
    [pscustomobject]@{ X = $r.L; Y = $r.T; W = $r.R - $r.L; H = $r.B - $r.T }
}
