# Reads and drives the application's **native** menu bar.
#
# Until 2026-08-25 the bar was drawn by the renderer, and the three harnesses that exercised it took
# their geometry from `TAILHAWK_DUMP_MENU_HITS` — a debug channel where the bar wrote out every rect
# it had drawn. The bar is now a real `HMENU` owned by Windows, that channel is deleted, and the
# harnesses that used it were failing with *"menu drew no entries"* against an application that had
# a perfectly good menu. This file is what replaces it.
#
# **It is a better source than the rect dump was, not merely a substitute.** The rect dump could
# only report what Tailhawk *believed* it had drawn; `GetMenuItemInfoW` reports what Windows
# actually holds — the label, the accelerator, the command id, and whether the item is greyed or
# ticked. A mismatch between the pure `menu_bar()` tree and the menu on screen is now findable,
# where before both sides of that comparison came from the same place.
#
# Dot-source it after `Screen.ps1`, which owns the process and foreground helpers:
#
#     . (Join-Path $PSScriptRoot 'Screen.ps1')
#     . (Join-Path $PSScriptRoot 'Menu.ps1')

if (-not ('Mnu' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class Mnu {
    [DllImport("user32.dll")] public static extern IntPtr GetMenu(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetSubMenu(IntPtr menu, int pos);
    [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")] public static extern uint GetMenuItemID(IntPtr menu, int pos);
    [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu, uint item, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetMenuStringW(IntPtr menu, uint item, StringBuilder buf, int max, uint flags);
    [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint item, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr hwnd, StringBuilder buf, int max);
    [DllImport("user32.dll")]
    public static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);

    public const uint WM_INITMENUPOPUP = 0x0117;
    public const uint WM_COMMAND       = 0x0111;

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

    public const uint BYPOSITION = 0x00000400;
    public const uint GRAYED     = 0x00000001;
    public const uint DISABLED   = 0x00000002;
    public const uint CHECKED    = 0x00000008;
    public const uint POPUP      = 0x00000010;
    public const uint SEPARATOR  = 0x00000800;

    public static string Text(IntPtr menu, int pos) {
        StringBuilder b = new StringBuilder(512);
        int n = GetMenuStringW(menu, (uint)pos, b, b.Capacity, BYPOSITION);
        return n > 0 ? b.ToString() : "";
    }

    // The class of whatever window currently has the foreground. `#32768` is the menu class:
    // while a popup is tracking, that window, not the application's, is in front.
    public static string ForegroundClass() {
        StringBuilder b = new StringBuilder(256);
        GetClassNameW(GetForegroundWindow(), b, b.Capacity);
        return b.ToString();
    }
}
'@
}

# One item of one popup, as Windows holds it. `Text` is the raw string including the `&` mnemonic
# marker and the tab before the accelerator; `Label` and `Accel` are that string taken apart.
function Read-MenuItem([IntPtr]$Menu, [int]$Pos) {
    $state = [Mnu]::GetMenuState($Menu, $Pos, [Mnu]::BYPOSITION)
    $text = [Mnu]::Text($Menu, $Pos)
    $parts = $text -split "`t", 2
    $label = $parts[0]
    $accel = if ($parts.Count -gt 1) { $parts[1] } else { '' }
    $mnemonic = ''
    if ($label -match '&(.)') { $mnemonic = $Matches[1] }

    [pscustomobject]@{
        Index     = $Pos
        Text      = $text
        Label     = $label -replace '&', ''
        Raw       = $label
        Accel     = $accel
        Mnemonic  = $mnemonic
        Id        = [Mnu]::GetMenuItemID($Menu, $Pos)
        Separator = ($state -band [Mnu]::SEPARATOR) -ne 0
        Popup     = ($state -band [Mnu]::POPUP) -ne 0
        Enabled   = ($state -band ([Mnu]::GRAYED -bor [Mnu]::DISABLED)) -eq 0
        Checked   = ($state -band [Mnu]::CHECKED) -ne 0
        Handle    = [Mnu]::GetSubMenu($Menu, $Pos)
    }
}

# Every item of one popup, in order, separators included — a caller that wants to click things
# must see them, because they are what makes the row pitch uneven.
function Read-MenuItems([IntPtr]$Menu) {
    $n = [Mnu]::GetMenuItemCount($Menu)
    if ($n -le 0) { return @() }
    @(0..($n - 1) | ForEach-Object { Read-MenuItem $Menu $_ })
}

# **A popup's items are stale until the application has been asked to refresh them.** Tailhawk
# builds the bar once and refills each popup on `WM_INITMENUPOPUP`, so a menu read straight out of
# a running window describes the moment the window was created: with a large file open and every
# command available, the first smoke run still reported `Close Tab`, `Copy` and `Find…` as greyed.
#
# Sending that message ourselves is what makes a *live* reading possible without touching the
# keyboard, and so without stealing the desktop from whoever is using it.
function Sync-MenuPopup([IntPtr]$Hwnd, $Heading) {
    if ($Heading.Handle -eq [IntPtr]::Zero) { return }
    [void][Mnu]::SendMessageW($Hwnd, [Mnu]::WM_INITMENUPOPUP, $Heading.Handle, [IntPtr]$Heading.Index)
}

# The whole bar: each heading with its popup refreshed and read. Throws if the window has no menu
# at all, which is the failure the drawn-bar era could not distinguish from an empty one.
#
# `-Stale` skips the refresh, for the one question that wants the unrefreshed answer: what the bar
# looks like before any popup has ever been opened.
function Read-MenuBar([IntPtr]$Hwnd, [switch]$Stale) {
    $bar = [Mnu]::GetMenu($Hwnd)
    if ($bar -eq [IntPtr]::Zero) { throw 'the window has no native menu (GetMenu returned NULL)' }
    $headings = Read-MenuItems $bar
    foreach ($h in $headings) {
        if (-not $Stale) {
            Sync-MenuPopup $Hwnd $h
            Start-Sleep -Milliseconds 40
        }
        $h | Add-Member -NotePropertyName Items -NotePropertyValue @(
            if ($h.Handle -ne [IntPtr]::Zero) { Read-MenuItems $h.Handle } else { @() }
        )
    }
    # Emitted, not wrapped. A `,$headings` here would hand `foreach ($h in Read-MenuBar ...)` a
    # single element - the whole array - and the caller would then read `.Label` as every heading's
    # label joined together. Callers that need an array take `@(Read-MenuBar ...)`.
    $headings
}

# `Find-MenuItem $bar File Open` — heading and item matched on their visible text, case-insensitively
# and by prefix, so `Open` finds `Open…` without the caller pasting an ellipsis into a script.
function Find-MenuItem($Bar, [string]$Heading, [string]$Item) {
    $h = $Bar | Where-Object { $_.Label -like "$Heading*" } | Select-Object -First 1
    if (-not $h) { throw "no menu heading matching '$Heading'" }
    $i = $h.Items | Where-Object { -not $_.Separator -and $_.Label -like "$Item*" } | Select-Object -First 1
    if (-not $i) { throw "no item matching '$Item' under '$($h.Label)'" }
    $i | Add-Member -NotePropertyName Heading -NotePropertyValue $h -PassThru
}

# Chooses an item without opening its menu, by posting the `WM_COMMAND` that choosing it sends.
#
# **The id comes from the live `HMENU`, which is what makes this faithful rather than a shortcut.**
# A script that posted an id from a table would only be testing its own table; reading the id off
# the menu Windows holds means the message is the one the menu itself would send, and the only
# thing skipped is Windows' popup tracking - which is not our code. It needs no foreground, so a
# harness built on it can run while someone is using the desktop.
#
# **Posted, never sent.** A command that raises a modal dialog does not return until that dialog
# closes, and a blocking `SendMessage` would then wait for a dialog only this script can dismiss -
# a deadlock that looks exactly like a hung application.
#
# `Invoke-MenuItem` is the one to use where the *menu path* is the thing under test.
function Send-MenuCommand([IntPtr]$Hwnd, $Item) {
    [void][Mnu]::PostMessageW($Hwnd, [Mnu]::WM_COMMAND, [IntPtr][int]$Item.Id, [IntPtr]::Zero)
}

# Opens a heading's popup by its mnemonic and waits for the menu window to take the foreground.
# `Alt+F` is not a keystroke the application handles — it is Windows entering menu mode — so the
# only honest confirmation that it worked is that a `#32768` is now in front.
function Open-Menu($Wsh, [string]$Mnemonic) {
    $Wsh.SendKeys("%$($Mnemonic.ToLower())")
    $null = Wait-For { [Mnu]::ForegroundClass() -eq '#32768' } "the $Mnemonic menu to open" 5
    Start-Sleep -Milliseconds 150
}

# Closes whatever menu is tracking. Two escapes: one leaves the popup, one leaves menu mode.
function Close-Menu($Wsh) {
    $Wsh.SendKeys('{ESC}')
    Start-Sleep -Milliseconds 120
    $Wsh.SendKeys('{ESC}')
    Start-Sleep -Milliseconds 200
}

# Chooses an item by walking to it with the arrow keys and pressing Enter, rather than by typing
# its mnemonic. **Mnemonics are not safe to invoke with**: two items in one popup may share a
# letter, and Windows then cycles the highlight instead of choosing — the script would report a
# command it never ran. The arrow walk counts separators, which is why `Read-MenuItems` keeps them.
function Invoke-MenuItem($Wsh, $Item) {
    Open-Menu $Wsh $Item.Heading.Mnemonic
    $steps = ($Item.Heading.Items | Where-Object { $_.Index -le $Item.Index -and -not $_.Separator }).Count
    for ($i = 0; $i -lt $steps; $i++) {
        $Wsh.SendKeys('{DOWN}')
        Start-Sleep -Milliseconds 40
    }
    $Wsh.SendKeys('{ENTER}')
    Start-Sleep -Milliseconds 500
}

# The screen rect of a popup item, from Windows, **while the popup is displayed**. This is the
# replacement for the rect dump's one real virtue: geometry that comes from the product rather
# than from a script's arithmetic about row pitch.
function Get-MenuItemRect([IntPtr]$Hwnd, $Item) {
    $r = New-Object Mnu+RECT
    if (-not [Mnu]::GetMenuItemRect($Hwnd, $Item.Heading.Handle, $Item.Index, [ref]$r)) {
        throw "GetMenuItemRect failed for '$($Item.Label)' - is the popup open?"
    }
    [pscustomobject]@{
        CX = [int](($r.L + $r.R) / 2)
        CY = [int](($r.T + $r.B) / 2)
    }
}

# Every popup whose items share a mnemonic letter. A duplicate is a real defect — the letter stops
# choosing and starts cycling — and it is invisible to the pure tests, which assert on labels.
function Find-DuplicateMnemonics($Bar) {
    foreach ($h in $Bar) {
        $dupes = $h.Items |
            Where-Object { $_.Mnemonic -ne '' } |
            Group-Object { $_.Mnemonic.ToLower() } |
            Where-Object { $_.Count -gt 1 }
        foreach ($d in $dupes) {
            [pscustomobject]@{
                Heading  = $h.Label
                Mnemonic = $d.Name
                Items    = ($d.Group | ForEach-Object { $_.Label }) -join ', '
            }
        }
    }
}
