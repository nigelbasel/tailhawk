# Drives §5's rules dialog through every path it has and asserts what each one did.
#
#   powershell tools/verify-rules.ps1
#
# **It never takes the foreground**, so it can run while the desktop is in use. Two things make
# that possible. The dialog is standard controls rather than a D3D swapchain, so `PrintWindow` with
# `PW_RENDERFULLCONTENT` renders it into a bitmap wherever it sits in the z-order. And every
# gesture is a message to the control that would have received it — a key to the list, a
# `WM_COMMAND` to a button — so nothing has to be clicked.
#
# **Do not reach for `LVM_SETITEMSTATE` to move the selection.** Its `lParam` is a pointer to an
# `LVITEM` in the *sender's* address space, and `comctl32` does not marshal list-view messages
# across a process boundary: the application dereferences a pointer it does not own and dies with
# `0xC000041D` inside `comctl32.dll`. That reads exactly like an application defect, and cost an
# hour on 2026-08-27 before the Application event log gave it away. A `WM_KEYDOWN` goes through the
# control's own input path and carries no pointer.
[CmdletBinding()]
param(
    [string]$Log = "$env:TEMP\tailhawk-verify-rules.log",
    [switch]$Shots
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')
. (Join-Path $PSScriptRoot 'Dialog.ps1')
Add-Type -AssemblyName System.Drawing

if (-not ('Rules' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Drawing;
using System.Runtime.InteropServices;
public static class Rules {
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="SendMessageW")]
    public static extern IntPtr SendTextW(IntPtr h, uint m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    // WM_GETTEXT, not GetWindowTextW: for a control owned by another process GetWindowText
    // returns nothing, while USER32 marshals WM_GETTEXT's buffer across the boundary.
    public static string Text(IntPtr h) {
        if (h == IntPtr.Zero) { return "<no control>"; }
        StringBuilder b = new StringBuilder(1024);
        SendTextW(h, 0x000D, (IntPtr)1024, b);
        return b.ToString();
    }
    public static void Key(IntPtr h, int vk) {
        SendMessageW(h, 0x0100, (IntPtr)vk, (IntPtr)0);
        SendMessageW(h, 0x0101, (IntPtr)vk, (IntPtr)0);
    }
    public static void Type(IntPtr h, string s) {
        foreach (char c in s) { SendMessageW(h, 0x0102, (IntPtr)c, (IntPtr)0); }
    }
    public static void Press(IntPtr dlg, int id) {
        SendMessageW(dlg, 0x0111, (IntPtr)id, GetDlgItem(dlg, id));
    }
    // For a button that raises something modal. `ChooseColorW` pumps its own message loop, so a
    // sent WM_COMMAND would not return until that loop ended — and the caller is what ends it.
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    public static void Post(IntPtr dlg, int id) {
        PostMessageW(dlg, 0x0111, (IntPtr)id, GetDlgItem(dlg, id));
    }
    public static Bitmap Shoot(IntPtr hwnd) {
        RECT r; GetWindowRect(hwnd, out r);
        Bitmap bmp = new Bitmap(r.R - r.L, r.B - r.T);
        using (Graphics g = Graphics.FromImage(bmp)) {
            IntPtr dc = g.GetHdc(); PrintWindow(hwnd, dc, 2); g.ReleaseHdc(dc);
        }
        return bmp;
    }
}
'@ -ReferencedAssemblies System.Drawing
}

# `dialog.rs`'s control ids.
$ID = @{ LIST = 180; ADD = 181; REMOVE = 182; UP = 183; DOWN = 184; NAME = 185; PATTERN = 186
         FG = 187; BG = 188; FGPICK = 189; BGPICK = 190; ENABLED = 191; ERROR = 195; SAVE = 196 }
$VK_UP = 0x26
$VK_DOWN = 0x28

if (-not (Test-Path $Log)) {
    $sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
    for ($i = 0; $i -lt 400; $i++) {
        $level = @('INFO', 'DEBUG', 'ERROR')[$i % 3]
        $sw.WriteLine("2026-08-27 09:14:0$($i % 10).117 $level  Api.Controller line $i handled in 12ms")
    }
    $sw.Close()
}

$failures = @()
function Check([string]$what, [bool]$ok, [string]$saw) {
    if ($ok) { Write-Host "  ok   $what" }
    else {
        Write-Host "  FAIL $what -- saw: $saw"
        $script:failures += $what
    }
}

Get-Process tailhawk -ErrorAction SilentlyContinue | Stop-Process -Force
$null = Wait-For { (Get-Process tailhawk -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0 } 'the previous instance to exit' 15

# **`Save` writes the personal tier, so the personal tier is moved somewhere disposable.**
# `%APPDATA%\Tailhawk\tailhawk.rules.toml` is the owner's own rule set — the one they curate and
# actually use — and a harness that presses Save would overwrite it. The application resolves that
# tier through `std::env::var_os("APPDATA")`, so pointing the *child process* at a scratch
# directory redirects it completely, with nothing to back up and nothing to restore if this script
# dies half way.
$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) 'tailhawk-verify-rules-appdata'
$personal = Join-Path $sandbox 'Tailhawk\tailhawk.rules.toml'
if (Test-Path $sandbox) { Remove-Item $sandbox -Recurse -Force }
$null = New-Item -ItemType Directory -Force -Path (Split-Path $personal)
@'
[[rule]]
name = "seeded"
pattern = "\\bERROR\\b"
fg = "#ff7a6b"

[[rule]]
name = "second"
pattern = "WARN"
bg = "#3b2e0f"
literal = true
enabled = false
'@ | ForEach-Object { [System.IO.File]::WriteAllText($personal, $_, [System.Text.UTF8Encoding]::new($false)) }
$env:APPDATA = $sandbox

# Not `Start-Tailhawk`: that waits for the foreground, which is exactly what this avoids.
$proc = Start-Process (Get-TailhawkExe) -ArgumentList $Log -PassThru
$null = Wait-For { $proc.Refresh(); $proc.MainWindowHandle -ne 0 } 'the window to open' 20
Start-Sleep -Milliseconds 900
$main = $proc.MainWindowHandle

function Field([IntPtr]$dlg, [int]$id) { [Rules]::Text([Rules]::GetDlgItem($dlg, $id)) }

try {
    Send-MenuCommand $main (Find-MenuItem (Read-MenuBar $main) 'Rules' 'Highlight rules')
    $dlg = Wait-Dialog $proc.Id 'Highlight rules' 10
    Start-Sleep -Milliseconds 600
    Check 'the editor is a real dialog, not a sheet drawn on the grid' ($dlg -ne [IntPtr]::Zero) 'no #32770'
    $list = [Rules]::GetDlgItem($dlg, $ID.LIST)

    $first = Field $dlg $ID.NAME
    Check 'it opens on a rule, with the fields filled' ($first.Length -gt 0) "name='$first'"

    # --- the selection repoints every field ---
    [Rules]::Key($list, $VK_DOWN)
    Start-Sleep -Milliseconds 400
    $second = Field $dlg $ID.NAME
    Check 'choosing another rule repoints the fields' ($second -ne $first) "name='$second'"
    [Rules]::Key($list, $VK_UP)
    Start-Sleep -Milliseconds 400
    Check 'and choosing back again returns them' ((Field $dlg $ID.NAME) -eq $first) (Field $dlg $ID.NAME)

    # --- §5: checked as you type, inline, never on OK ---
    $edit = [Rules]::GetDlgItem($dlg, $ID.PATTERN)
    [void][Rules]::SendMessageW($edit, 0x00B1, [IntPtr]0, [IntPtr](-1))   # EM_SETSEL all
    [Rules]::Type($edit, 'ERROR(')
    Start-Sleep -Milliseconds 600
    $typed = Field $dlg $ID.PATTERN
    # **The caret regression.** A refresh that rewrites the control being typed into sends the
    # caret back to the start, and the second character then lands in front of the first. Six
    # characters in the order they were typed is the whole assertion.
    Check 'what is typed arrives in the order it was typed' ($typed -eq 'ERROR(') "pattern='$typed'"
    $err = Field $dlg $ID.ERROR
    Check 'a broken pattern says so beside itself' ($err -match 'unclosed group') "error='$err'"
    Check 'and says it on the one line it has' (-not $err.Contains("`n")) "error='$err'"

    [Rules]::Type($edit, ')')
    Start-Sleep -Milliseconds 600
    Check 'fixing it clears the complaint' ((Field $dlg $ID.ERROR).Length -eq 0) (Field $dlg $ID.ERROR)
    Check 'and the fix lands at the caret' ((Field $dlg $ID.PATTERN) -eq 'ERROR()') (Field $dlg $ID.PATTERN)

    # --- the verbs ---
    [Rules]::Press($dlg, $ID.ADD)
    Start-Sleep -Milliseconds 500
    Check 'Add opens a rule whose only complaint is the pattern it has not got' `
        ((Field $dlg $ID.ERROR) -match 'no pattern') (Field $dlg $ID.ERROR)
    Check 'and gives it a colour, so that is not a second complaint' `
        ((Field $dlg $ID.FG).StartsWith('#')) (Field $dlg $ID.FG)

    [Rules]::Press($dlg, $ID.UP)
    Start-Sleep -Milliseconds 400
    Check 'Move up carries the selection with the rule' `
        ((Field $dlg $ID.ERROR) -match 'no pattern') (Field $dlg $ID.ERROR)

    [Rules]::Press($dlg, $ID.REMOVE)
    Start-Sleep -Milliseconds 400
    Check 'Remove drops it and selects a neighbour' `
        ((Field $dlg $ID.NAME).Length -gt 0) (Field $dlg $ID.NAME)

    # --- the colour picker, and the byte order underneath it ---
    #
    # **Posted, not sent.** `ChooseColorW` pumps its own modal loop, so a `SendMessage` that
    # started it would not return until it closed — and this script is what closes it.
    #
    # Pressing OK without touching anything returns the colour the dialog was *seeded* with, which
    # is the assertion worth having: `COLORREF` is `0x00bbggrr` and `#rrggbb` is the other way
    # round, so a round trip that came back with red and blue swapped would be invisible in a
    # screenshot of the swatch but obvious here.
    $before = Field $dlg $ID.FG
    [Rules]::Post($dlg, $ID.FGPICK)
    # The picker's caption is Windows', not ours, so it is found by *being a second dialog of this
    # process* rather than by a title this script would have to guess — and guessing wrong would
    # report a missing dialog that is plainly on screen.
    $picker = [IntPtr]::Zero
    try {
        $null = Wait-For {
            $script:picker = @([Dlg]::Windows($proc.Id) | Where-Object { $_ -like '#32770|*' -and $_ -notlike '*Highlight rules' })
            $script:picker.Count -gt 0
        } 'the colour picker' 8
    } catch { }
    $raised = @([Dlg]::Windows($proc.Id) | Where-Object { $_ -like '#32770|*' -and $_ -notlike '*Highlight rules' })
    $picker = if ($raised) { Get-Dialog $proc.Id ($raised[0] -replace '^#32770\|', '') } else { [IntPtr]::Zero }
    Check 'Text... opens the standard colour picker' ($picker -ne [IntPtr]::Zero) `
        "this process's dialogs: $([Dlg]::Windows($proc.Id) -join ' / ')"
    if ($picker -ne [IntPtr]::Zero) {
        [void][Rules]::SendMessageW($picker, 0x0111, [IntPtr]1, [IntPtr]0)   # IDOK
        Start-Sleep -Milliseconds 600
        Check 'and OK on the seeded colour returns it unchanged' `
            ((Field $dlg $ID.FG) -eq $before) "was '$before', now '$(Field $dlg $ID.FG)'"
    }

    # --- Save writes the personal tier ---
    $marker = "verified-$([System.Guid]::NewGuid().ToString('N').Substring(0, 8))"
    [void][Rules]::SendMessageW([Rules]::GetDlgItem($dlg, $ID.NAME), 0x00B1, [IntPtr]0, [IntPtr](-1))
    [Rules]::Type([Rules]::GetDlgItem($dlg, $ID.NAME), $marker)
    Start-Sleep -Milliseconds 400
    [Rules]::Press($dlg, $ID.SAVE)
    Start-Sleep -Milliseconds 900
    $written = if (Test-Path $personal) { Get-Content $personal -Raw } else { '' }
    Check 'Save writes the personal tier' ($written -match [regex]::Escape($marker)) `
        "the file does not carry the name that was typed"

    # --- Close leaves the on-disk rules in force, and says so when the set was unsaved ---
    [void][Rules]::SendMessageW([Rules]::GetDlgItem($dlg, $ID.PATTERN), 0x00B1, [IntPtr]0, [IntPtr](-1))
    [Rules]::Type([Rules]::GetDlgItem($dlg, $ID.PATTERN), 'THROWN-AWAY')
    Start-Sleep -Milliseconds 400
    Check 'the edit to be discarded reached the dialog' `
        ((Field $dlg $ID.PATTERN) -eq 'THROWN-AWAY') (Field $dlg $ID.PATTERN)
    [Rules]::Press($dlg, 2)   # IDCANCEL — the Close button
    Start-Sleep -Milliseconds 250
    $proc.Refresh()
    $justAfter = $proc.MainWindowTitle
    Start-Sleep -Milliseconds 900
    $proc.Refresh()
    if ($justAfter -match 'unsaved' -and $proc.MainWindowTitle -notmatch 'unsaved') {
        Write-Host "  NOTE the message appeared and was then overwritten within a second"
    }
    Check 'Close says so when the set was left unsaved' `
        ($proc.MainWindowTitle -match 'unsaved') $proc.MainWindowTitle
    $after = if (Test-Path $personal) { Get-Content $personal -Raw } else { '' }
    Check 'and does not write the changes it threw away' `
        (-not ($after -match 'THROWN-AWAY')) 'the discarded edit reached the file'

    # Re-open for the last check, since Close took it down.
    Send-MenuCommand $main (Find-MenuItem (Read-MenuBar $main) 'Rules' 'Highlight rules')
    $dlg = Wait-Dialog $proc.Id 'Highlight rules' 10
    Start-Sleep -Milliseconds 500

    if ($Shots) {
        $bmp = [Rules]::Shoot($dlg)
        $bmp.Save((Join-Path $PSScriptRoot '..\rules-dialog.png'), [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
    }

    # --- §12 gives the editor one key that both reaches it and dismisses it ---
    Send-MenuCommand $main (Find-MenuItem (Read-MenuBar $main) 'Rules' 'Highlight rules')
    Start-Sleep -Milliseconds 800
    Check 'choosing it again takes it down' ((Get-Dialog $proc.Id 'Highlight rules') -eq [IntPtr]::Zero) 'still up'

    $proc.Refresh()
    Check 'and the application is still running' (-not $proc.HasExited) 'it exited'
} finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}

Write-Host ''
if ($failures.Count -eq 0) {
    Write-Host 'PASS: the rules dialog answers on every path'
} else {
    Write-Host "FAIL: $($failures -join '; ')"
    exit 1
}
