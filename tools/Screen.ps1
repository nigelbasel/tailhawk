# Shared by the verify-*.ps1 harnesses: start the shipped binary, put it in the foreground, read
# its client area off the screen and count colours in it. Dot-source it:
#
#   . (Join-Path $PSScriptRoot 'Screen.ps1')
#
# Kept here rather than in each harness so a fix to the focus dance or the DPI handling is made
# once. Nothing in this file knows what a harness is looking for.

Add-Type -AssemblyName System.Drawing

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

# **The document's facts are in the status bar, not the title** — UX-REVIEW finding 12 moved them
# there on 2026-09-15, and every harness that waited on the title for "lines" or "of N" reads this
# instead. `WM_GETTEXT` is one of the messages Windows marshals between processes, so a string buffer
# in this process is safe to hand to a control in another; a status bar answers it with part zero.
if (-not ('StatusText' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;

// **Every pane, joined — not WM_GETTEXT.** The bar carried one composed sentence until
// 2026-09-21 and now carries eight parts, and WM_GETTEXT answers for the first of them only.
// A harness reading that would have seen the message pane and none of the facts, which is
// exactly the half a readiness check needs. SB_GETPARTS with a null array reports the count.
public static class StatusText {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    const uint SB_GETPARTS = 0x0406;
    const uint SB_GETTEXTW = 0x040D;

    public static IntPtr Bar(IntPtr window) {
        IntPtr bar = IntPtr.Zero;
        EnumChildWindows(window, (h, l) => {
            StringBuilder c = new StringBuilder(64);
            GetClassNameW(h, c, 64);
            if (c.ToString() != "msctls_statusbar32") { return true; }
            bar = h;
            return false;
        }, IntPtr.Zero);
        return bar;
    }

    public static string Of(IntPtr window) {
        IntPtr bar = Bar(window);
        if (bar == IntPtr.Zero) { return ""; }
        int parts = (int)SendMessageW(bar, SB_GETPARTS, IntPtr.Zero, IntPtr.Zero);
        if (parts <= 0) { parts = 1; }
        StringBuilder all = new StringBuilder();
        for (int i = 0; i < parts; i++) {
            StringBuilder text = new StringBuilder(1024);
            SendMessageW(bar, SB_GETTEXTW, (IntPtr)i, text);
            string one = text.ToString();
            if (one.Length == 0) { continue; }
            if (all.Length > 0) { all.Append(" | "); }
            all.Append(one);
        }
        return all.ToString();
    }
}
'@
}

function Get-StatusText([System.Diagnostics.Process]$Proc) {
    $Proc.Refresh()
    if ($Proc.MainWindowHandle -eq 0) { return '' }
    [StatusText]::Of($Proc.MainWindowHandle)
}

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

# The shipped binary, or nothing: a harness that quietly ran a debug build would be measuring the
# wrong thing.
function Get-TailhawkExe {
    $root = Split-Path -Parent $PSScriptRoot
    $exe = Join-Path $root 'target\release\tailhawk.exe'
    if (-not (Test-Path $exe)) { throw "build it first: cargo build --release   ($exe is missing)" }
    $exe
}

# Starts the binary on a file, waits for its window, and puts that window in the foreground so
# SendKeys reaches it. Returns the process; its MainWindowHandle is the window.
function Start-Tailhawk([string]$Log) {
    $proc = Start-Process (Get-TailhawkExe) -ArgumentList $Log -PassThru
    # **`Line N of M`, not `lines`.** The bar said `1192 lines, 419501 bytes` until 2026-09-21;
    # the position pane says `Line 1,192 of 419,501` and the byte count is gone. Matching the
    # pane that only exists once a document is open is still what "the window opened" means.
    $null = Wait-For { (Get-StatusText $proc) -match 'Line [\d,]+ of' } 'the window to open'
    $hwnd = $proc.MainWindowHandle
    Write-Host "opened: $($proc.MainWindowTitle) | $(Get-StatusText $proc)"

    # AppActivate's return value says the request was made, not that it was honoured; the keys go
    # to whichever window is foreground when they are sent, so wait for that to be ours. A bare
    # Alt releases the foreground lock that otherwise keeps a busy user's window on top -- but it
    # is sent only while some *other* window is in front, because an Alt delivered to our own
    # window puts it into menu mode and every key after it is swallowed.
    $wsh = New-Object -ComObject WScript.Shell
    try {
        $null = Wait-For {
            if ([Shot]::GetForegroundWindow() -eq $hwnd) { return $true }
            $wsh.SendKeys('%')
            $wsh.AppActivate($proc.Id) | Out-Null
            Start-Sleep -Milliseconds 100
            [Shot]::GetForegroundWindow() -eq $hwnd
        } 'the window to take the foreground (is someone using the desktop?)' 10
    } catch {
        # The caller has no handle yet, so this is the only place that can close the window.
        if (-not $proc.HasExited) { $proc.Kill() }
        throw
    }
    Start-Sleep -Milliseconds 400
    $proc
}

# Escapes a string so SendKeys delivers it as itself: braces, parens and the metacharacters.
function ConvertTo-SendKeys([string]$Text) {
    $Text -replace '([+^%~(){}\[\]])', '{$1}'
}

# A colour as the render target writes it: the float channels of a `[f32; 4]` times 255.
function ConvertFrom-Rgbf([double]$R, [double]$G, [double]$B) {
    @([int][math]::Round($R * 255), [int][math]::Round($G * 255), [int][math]::Round($B * 255))
}
