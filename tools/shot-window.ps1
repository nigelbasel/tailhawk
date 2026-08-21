# Captures the running Tailhawk window, and always puts the z-order back.
#
# **Why this file exists.** A D3D11 swapchain cannot be captured with `PrintWindow` — GDI sees
# nothing and saves a blank bitmap — so the window has to be genuinely on screen and grabbed with
# `CopyFromScreen`. Getting it on screen is the awkward part: `SetForegroundWindow` is refused
# while the desktop is in use, so the reliable lever is `HWND_TOPMOST`.
#
# That lever is a trap, and it caught this project: a capture pinned the window topmost, the script
# ended, and the flag stayed. Tailhawk then sat above every other window on the owner's desktop for
# the rest of the day and looked like an application defect. It was not — the app's only
# `SetWindowPos` is its `WM_DPICHANGED` handler, which passes `SWP_NOZORDER` precisely so it never
# touches z-order.
#
# So the rule here: **whatever sets topmost must clear it, in a `finally`, even if the capture
# throws.** Nothing else in this repo may pin a window without doing the same.

[CmdletBinding()]
param(
    [string]$Shot = "$env:TEMP\tailhawk-window.png",
    [int]$X = 60,
    [int]$Y = 60,
    [int]$Width = 1000,
    [int]$Height = 620,
    [int]$SettleMs = 1200,
    # Crop, relative to the window's top-left. Zero width or height means the whole window.
    [int]$CropX = 0,
    [int]$CropY = 0,
    [int]$CropW = 0,
    [int]$CropH = 0
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @'
using System;
using System.Runtime.InteropServices;
public class ThWin {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int t, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int index);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@

$proc = Get-Process tailhawk -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { throw 'no Tailhawk window is open' }
$h = $proc.MainWindowHandle

$TOPMOST = [IntPtr](-1)
$NOTOPMOST = [IntPtr](-2)
$SHOWWINDOW = 0x40
$NOMOVE_NOSIZE_NOACTIVATE = 0x13
$GWL_EXSTYLE = -20
$WS_EX_TOPMOST = 0x8

# Remember whether it was *already* topmost, so a window the owner pinned themselves stays pinned.
$wasTopmost = ([ThWin]::GetWindowLong($h, $GWL_EXSTYLE) -band $WS_EX_TOPMOST) -ne 0

try {
    [void][ThWin]::SetWindowPos($h, $TOPMOST, $X, $Y, $Width, $Height, $SHOWWINDOW)
    Start-Sleep -Milliseconds $SettleMs

    $r = New-Object ThWin+RECT
    [void][ThWin]::GetWindowRect($h, [ref]$r)

    $srcX = $r.Left + $CropX
    $srcY = $r.Top + $CropY
    $w = if ($CropW -gt 0) { $CropW } else { $r.Right - $r.Left }
    $t = if ($CropH -gt 0) { $CropH } else { $r.Bottom - $r.Top }

    $bmp = New-Object System.Drawing.Bitmap($w, $t)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($srcX, $srcY, 0, 0, (New-Object System.Drawing.Size($w, $t)))
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    "saved $Shot ($w x $t)"
}
finally {
    # **The whole point of the file.** Restore the z-order the window had, whatever happened above.
    if (-not $wasTopmost) {
        [void][ThWin]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, $NOMOVE_NOSIZE_NOACTIVATE)
    }
}
