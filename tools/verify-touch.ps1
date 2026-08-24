# Drives the shipped binary through a real touch flick and proves the view coasts after the lift.
#
# `UI-DESIGN.md` §12 asks for inertial scrolling and says why: discrete steps are what make a
# hand-rolled Win32 app feel homemade next to Edge and Terminal. `fling.rs` unit-tests the physics,
# but the physics being right is not the same as the messages arriving, the contact being routed to
# the rows, or the timer running -- and none of that can be reached from a test that has no window.
# So this injects an actual contact at actual screen coordinates with `InjectTouchInput` and reads
# the answer off the screen.
#
# The measurement is deliberately not "did it scroll". It is **did it keep scrolling after the
# finger came off**, which is the only part that is new, and it is checked against a control: a slow
# drag must come to rest where it is released, and a flick must not.
#
#   powershell tools/verify-touch.ps1
#   powershell tools/verify-touch.ps1 -Lines 400000 -KeepWindow
[CmdletBinding()]
param(
    [int]$Lines = 200000,
    [string]$Log = "$env:TEMP\tailhawk-verify-touch.log",
    [string]$Shot = "$env:TEMP\tailhawk-verify-touch.png",
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class Touch {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool InitializeTouchInjection(uint maxCount, uint dwMode);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool InjectTouchInput(uint count, [In] POINTER_TOUCH_INFO[] contacts);

    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINTER_INFO {
        public uint pointerType;
        public uint pointerId;
        public uint frameId;
        public uint pointerFlags;
        public IntPtr sourceDevice;
        public IntPtr hwndTarget;
        public POINT ptPixelLocation;
        public POINT ptHimetricLocation;
        public POINT ptPixelLocationRaw;
        public POINT ptHimetricLocationRaw;
        public uint dwTime;
        public uint historyCount;
        public int InputData;
        public uint dwKeyStates;
        public ulong PerformanceCount;
        public int ButtonChangeType;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINTER_TOUCH_INFO {
        public POINTER_INFO pointerInfo;
        public uint touchFlags;
        public uint touchMask;
        public RECT rcContact;
        public RECT rcContactRaw;
        public uint orientation;
        public uint pressure;
    }

    public const uint PT_TOUCH = 2;
    public const uint DOWN   = 0x00010000 | 0x00000002 | 0x00000004; // DOWN | INRANGE | INCONTACT
    public const uint UPDATE = 0x00020000 | 0x00000002 | 0x00000004; // UPDATE | INRANGE | INCONTACT
    public const uint UP     = 0x00040000;                           // UP

    // TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE
    public const uint MASK = 0x1 | 0x2 | 0x4;

    public static POINTER_TOUCH_INFO Contact(int x, int y, uint flags) {
        POINTER_TOUCH_INFO t = new POINTER_TOUCH_INFO();
        t.pointerInfo.pointerType = PT_TOUCH;
        t.pointerInfo.pointerId = 0;
        t.pointerInfo.pointerFlags = flags;
        t.pointerInfo.ptPixelLocation.X = x;
        t.pointerInfo.ptPixelLocation.Y = y;
        t.touchFlags = 0;
        t.touchMask = MASK;
        t.rcContact.L = x - 4; t.rcContact.R = x + 4;
        t.rcContact.T = y - 4; t.rcContact.B = y + 4;
        t.orientation = 90;
        t.pressure = 512;
        return t;
    }

    public static void Send(int x, int y, uint flags) {
        POINTER_TOUCH_INFO[] one = new POINTER_TOUCH_INFO[1];
        one[0] = Contact(x, y, flags);
        if (!InjectTouchInput(1, one)) {
            throw new Exception("InjectTouchInput failed: " + Marshal.GetLastWin32Error());
        }
    }
}
'@

# A fingerprint of the rows, so "did anything move" is a comparison and not a judgement.
#
# **The chrome is deliberately excluded.** The status bar carries the live frame-timing readout and
# the command bar carries the follow state, so a fingerprint of the whole client area changes when a
# frame is drawn rather than when the view moves -- which is the opposite of the question. An
# earlier draft sampled everything and reported a coast that "never stopped" 2.5 s after the lift,
# when what had actually changed between the two captures was `worst 51.6 ms`.
function Get-Frame([IntPtr]$Hwnd) {
    $bmp = [Shot]::Client($Hwnd)
    try {
        $sb = New-Object System.Text.StringBuilder
        $top = 100
        $bottom = $bmp.Height - 40
        for ($y = $top; $y -lt $bottom; $y += 7) {
            for ($x = 0; $x -lt $bmp.Width; $x += 11) {
                [void]$sb.Append($bmp.GetPixel($x, $y).ToArgb().ToString('x'))
            }
        }
        $md5 = [System.Security.Cryptography.MD5]::Create()
        [BitConverter]::ToString($md5.ComputeHash([Text.Encoding]::ASCII.GetBytes($sb.ToString())))
    } finally { $bmp.Dispose() }
}

# One gesture: down, `Steps` moves of `Dy` pixels each `Pause` ms apart, then up where it stopped.
# A big `Dy` with a small `Pause` is a flick; a small `Dy` with a big `Pause` is a drag.
function Send-Gesture([int]$X, [int]$Y, [int]$Dy, [int]$Steps, [int]$Pause) {
    [Touch]::Send($X, $Y, [Touch]::DOWN)
    $at = $Y
    for ($i = 0; $i -lt $Steps; $i++) {
        Start-Sleep -Milliseconds $Pause
        $at += $Dy
        [Touch]::Send($X, $at, [Touch]::UPDATE)
    }
    [Touch]::Send($X, $at, [Touch]::UP)
}

Write-Host "writing $Lines lines to $Log"
$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt $Lines; $i++) {
    $sw.WriteLine("2026-08-21 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
}
$sw.Close()

# Mode 3 is TOUCH_FEEDBACK_NONE. The default draws Windows own touch indicator at the contact,
# which is painted over our window and fades after the lift -- `CopyFromScreen` captures it, so the
# rows appear to still be changing several hundred ms after a gesture ends. That is what made the
# first drafts of this harness intermittently report a coast that never stopped.
if (-not [Touch]::InitializeTouchInjection(1, 3)) {
    throw "InitializeTouchInjection failed: $([ComponentModel.Win32Exception]::new([Runtime.InteropServices.Marshal]::GetLastWin32Error()).Message)"
}

$proc = Start-Tailhawk $Log
try {
    $hwnd = $proc.MainWindowHandle
    $r = New-Object Shot+RECT
    [void][Shot]::GetClientRect($hwnd, [ref]$r)
    $o = New-Object Shot+POINT
    [void][Shot]::ClientToScreen($hwnd, [ref]$o)

    # Well down the rows, clear of the menu bar, the command bar and the header band.
    #
    # **Every gesture here drags the finger *down*, which scrolls back up the file.** A log opens at
    # its end, so a contact dragged upward asks a view that is already on its last line to go
    # further -- it is pinned, nothing moves, and all four checks below pass or fail on whether the
    # screen changed. An earlier draft of this harness did exactly that and reported "no inertia"
    # against a build whose inertia was fine.
    $x = $o.X + [int](($r.R - $r.L) / 2)
    $y = $o.Y + [int](($r.B - $r.T) * 0.62)
    Write-Host "contact at $x,$y in a $($r.R - $r.L) x $($r.B - $r.T) client area"

    # --- Control: a slow drag has to stop dead where it is let go. ---------------------------
    Send-Gesture -X $x -Y $y -Dy 6 -Steps 8 -Pause 60
    Start-Sleep -Milliseconds 120
    $restA = Get-Frame $hwnd
    Start-Sleep -Milliseconds 500
    $restB = Get-Frame $hwnd
    if ($restA -ne $restB) {
        throw "a slow drag coasted -- it must come to rest where the finger leaves it"
    }
    Write-Host "slow drag: at rest 500 ms after the lift  OK"

    # --- The flick: the view has to still be moving after the finger is gone. ----------------
    Send-Gesture -X $x -Y $y -Dy 55 -Steps 7 -Pause 8
    $flickA = Get-Frame $hwnd
    Start-Sleep -Milliseconds 250
    $flickB = Get-Frame $hwnd
    if ($flickA -eq $flickB) {
        throw "the view stopped with the finger -- no inertia (UI-DESIGN.md 12)"
    }
    Write-Host "flick: still moving 250 ms after the lift  OK"

    # --- And it has to end. An exponential tail still has to reach a standstill. -------------
    Start-Sleep -Milliseconds 2500
    $endA = Get-Frame $hwnd
    Start-Sleep -Milliseconds 500
    $endB = Get-Frame $hwnd
    if ($endA -ne $endB) {
        throw "the coast never stopped -- it is still moving 2.5 s after the lift"
    }
    Write-Host "flick: come to rest by 2.5 s  OK"

    # --- A finger put down on a moving view stops it, which is how a coast is cancelled. -----
    Send-Gesture -X $x -Y $y -Dy 55 -Steps 7 -Pause 8
    Start-Sleep -Milliseconds 60
    [Touch]::Send($x, $y, [Touch]::DOWN)
    Start-Sleep -Milliseconds 40
    [Touch]::Send($x, $y, [Touch]::UP)
    Start-Sleep -Milliseconds 150
    $stopA = Get-Frame $hwnd
    Start-Sleep -Milliseconds 400
    $stopB = Get-Frame $hwnd
    if ($stopA -ne $stopB) {
        throw "a tap did not stop the coast -- a finger on a moving view has to catch it"
    }
    Write-Host "tap during a coast: caught it  OK"

    $bmp = [Shot]::Client($hwnd)
    try { $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png) } finally { $bmp.Dispose() }
    Write-Host "saved $Shot"
    Write-Host 'verify-touch: PASS'
} finally {
    if (-not $KeepWindow -and -not $proc.HasExited) { $proc.Kill() }
}
