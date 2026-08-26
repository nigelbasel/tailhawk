# Chooses every item in every menu and reports what each one did.
#
# `File > Open…` did nothing when chosen for a month while working perfectly from the keyboard and
# the palette. No unit test caught it, and none could have: the tests and the keyboard go through
# the same `Shell::run`, and what was missing was in the path after it. The only thing that finds a
# bug of that shape is choosing the thing.
#
# What it can decide on its own is deliberately modest - it reports **observable effect**, not
# correctness. An enabled item that changes nothing at all is the signature of the Open bug and is
# flagged `SUSPECT`; whether an item did the *right* thing is a human's judgement from the log it
# prints.
#
#   powershell tools/verify-menus.ps1
#   powershell tools/verify-menus.ps1 -Menu File
#
# **What changed on 2026-08-26.** The menu is a real `HMENU` now, so the old machinery - a rect
# dump out of `TAILHAWK_DUMP_MENU_HITS` and a touch tap at the centre of each rect - is gone along
# with the bar it measured, and this script had been failing with *"menu drew no entries"* against
# an application whose menus were fine. Two things follow.
#
# The **item list comes from Windows** rather than from a table here, so a menu that grows an entry
# grows this sweep with it, and the old hazard of clicking `Exit` while reporting a separator
# cannot arise: separators are marked as such by `GetMenuState`.
#
# **Every suspect is asked twice.** The client area is captured *off the screen* — a D3D11
# swapchain cannot be rendered into a bitmap with `PrintWindow`, which is why `shot-window.ps1` has
# to pin the window topmost — so anything overlapping the window for the moment of a capture turns
# a real change into "nothing happened". Two consecutive runs on 2026-08-26 disagreed about exactly
# the items whose whole effect was drawn, and about nothing else. So an item that reports nothing
# is run again in another fresh window, and only a *second* silent reading is carried to the
# summary. One that moves on the retry says so, because a reading that changes between two
# identical runs is itself worth seeing.
#
# And the **effect is observed on four surfaces at once**: the window title, which carries the
# file, format, follow state, filters and sort; the set of top-level windows, which catches every
# dialog; and a digest of the menu itself, which catches a tick flipping or an item going grey.
# The old version watched the title alone and would have called a check-mark change "no effect".
[CmdletBinding()]
param(
    [string]$Menu = '',
    [string]$Log = "$env:TEMP\tailhawk-verify-menus.log"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')
. (Join-Path $PSScriptRoot 'Dialog.ps1')

# Items this sweep will not choose, and why. **Nothing is skipped silently** - each is printed as
# it is passed over, because a sweep that quietly drops rows reads as coverage it does not have.
$SKIP = @{
    'Exit'                = 'ends the process, so nothing after it could run'
    'Close Tab'           = 'leaves no document, so every item after it would be greyed'
    'Clear recent files'  = 'destroys real history; verify-recent.ps1 covers it against a fixture'
}

if (-not (Test-Path $Log)) {
    $sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
    for ($i = 0; $i -lt 5000; $i++) {
        $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
    }
    $sw.Close()
}

# The surfaces an effect can show on. **The pixels are one of them, and they are what makes the
# sweep honest for a viewer**: a bookmark lands in the gutter, `Go to top` moves the scroll, and
# neither reaches the window title - without the client area those read as commands that did
# nothing, which is exactly the false alarm this script exists to avoid raising.
function Get-Surfaces($Proc, [switch]$NoShot) {
    $Proc.Refresh()
    $digest = (@(Read-MenuBar $Proc.MainWindowHandle) | ForEach-Object {
        $_.Items | ForEach-Object { "$($_.Label):$($_.Enabled):$($_.Checked)" }
    }) -join '|'
    [pscustomobject]@{
        Title   = $Proc.MainWindowTitle
        # Dialogs only. Listing every window would include the main one, whose *title* is already
        # the surface above - so the two would move together and one of them would be telling the
        # reader nothing.
        Windows = (@([Dlg]::Windows($Proc.Id) | Where-Object { $_ -like '#32770|*' }) | Sort-Object) -join ' / '
        Menu    = $digest
        Shot    = if ($NoShot) { $null } else { [Shot]::Client($Proc.MainWindowHandle) }
    }
}

function Remove-Surfaces($S) {
    if ($S -and $S.Shot) { $S.Shot.Dispose() }
}

# **The client area is captured off the screen, so the window has to be in front of it.** Closing a
# modal dialog does not reliably hand the foreground back, and a capture taken while something else
# is on top is identical before and after - which reads as "this command did nothing". Every
# SUSPECT left standing in the first calibrated run was an item chosen straight after a dialog, and
# two of them were the command palette and the rules editor: surfaces the application *draws*, so
# the one thing that could ever have shown them was the pixels.
#
# A bare `AppActivate` is refused: Windows only lets the process that owns the foreground, or that
# received the last input, take it. `Screen.ps1` solves the same problem when it starts the
# application, and the trick is a bare `Alt` first, which releases the lock. It is sent only while
# some *other* window is in front, because an `Alt` delivered to Tailhawk's own window puts it into
# menu mode and swallows everything after it.
function Restore-Foreground($Proc) {
    if ([Mnu]::GetForegroundWindow() -eq $Proc.MainWindowHandle) { return $true }
    $wsh = New-Object -ComObject WScript.Shell
    try {
        $null = Wait-For {
            if ([Mnu]::GetForegroundWindow() -eq $Proc.MainWindowHandle) { return $true }
            $wsh.SendKeys('%')
            $wsh.AppActivate($Proc.Id) | Out-Null
            Start-Sleep -Milliseconds 150
            [Mnu]::GetForegroundWindow() -eq $Proc.MainWindowHandle
        } 'the window to come back to the front' 6
        $true
    } catch {
        Write-Host '  (could not restore the foreground - the pixel reading is dropped for this item)'
        $false
    }
}

# **The pixels are compared with a tolerance, and the tolerance is measured rather than guessed.**
# The status bar carries live frame timings, so an idle window is never twice identical and an
# exact comparison marks every surface as restless - which drops the one surface that can see a
# drawn overlay. Calibration records how much an idle window churns; anything comfortably above
# that is a real change.
if (-not ('Pix' -as [type])) {
    Add-Type @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class Pix {
    // Differing pixels between two captures of the same window. Sizes that disagree count as
    // wholly different, which is the honest answer when the window has been resized.
    public static int Diff(Bitmap a, Bitmap b) {
        if (a.Width != b.Width || a.Height != b.Height) { return int.MaxValue; }
        Rectangle all = new Rectangle(0, 0, a.Width, a.Height);
        BitmapData da = a.LockBits(all, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        BitmapData db = b.LockBits(all, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        int n = 0;
        try {
            byte[] pa = new byte[da.Stride * da.Height];
            byte[] pb = new byte[db.Stride * db.Height];
            Marshal.Copy(da.Scan0, pa, 0, pa.Length);
            Marshal.Copy(db.Scan0, pb, 0, pb.Length);
            for (int i = 0; i < pa.Length; i += 4) {
                if (pa[i] != pb[i] || pa[i + 1] != pb[i + 1] || pa[i + 2] != pb[i + 2]) { n++; }
            }
        } finally { a.UnlockBits(da); b.UnlockBits(db); }
        return n;
    }
}
'@ -ReferencedAssemblies System.Drawing
}

$TEXT_SURFACES = @('Title', 'Windows', 'Menu')

function Compare-Surfaces($Before, $After, $Watch, [int]$Tolerance) {
    $moved = @($Watch | Where-Object { $_ -ne 'Pixels' -and $Before.$_ -ne $After.$_ })
    if ($Watch -contains 'Pixels' -and $Before.Shot -and $After.Shot) {
        if ([Pix]::Diff($Before.Shot, $After.Shot) -gt $Tolerance) { $moved += 'Pixels' }
    }
    @($moved)
}

# **Which surfaces hold still when nothing is done to them, and how much the pixels churn anyway.**
# A surface that changes on its own would mark every item as having had an effect - the opposite
# failure to the one being hunted, and a quieter one. Sampling the idle application first says
# which surfaces can be trusted for this run; the rest are dropped and said out loud.
function Measure-Calibration($Proc) {
    $samples = 1..4 | ForEach-Object { Start-Sleep -Milliseconds 700; Get-Surfaces $Proc }
    $stable = @($TEXT_SURFACES | Where-Object {
        $name = $_
        ($samples | ForEach-Object { $_.$name } | Sort-Object -Unique | Measure-Object).Count -eq 1
    })
    $churn = 0
    for ($i = 1; $i -lt $samples.Count; $i++) {
        $d = [Pix]::Diff($samples[$i - 1].Shot, $samples[$i].Shot)
        if ($d -gt $churn) { $churn = $d }
    }
    $samples | ForEach-Object { Remove-Surfaces $_ }
    [pscustomobject]@{
        Watch     = @($stable + 'Pixels')
        Stable    = $stable
        Churn     = $churn
        Tolerance = [Math]::Max($churn * 4, 400)
    }
}
# **Wait for the last instance to be gone, not merely asked to go.** Tailhawk is single-instance:
# a launch while a dying process still holds the claim hands the file to that process and exits
# without ever showing a window, and the sweep then fails on the *next* item with "timed out
# waiting for the window to open". A fixed sleep is what let this through; the condition is that
# no process is left.
function Wait-NoTailhawk {
    Get-Process tailhawk -ErrorAction SilentlyContinue | Stop-Process -Force
    $null = Wait-For {
        (Get-Process tailhawk -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0
    } 'the previous instance to exit' 15
    Start-Sleep -Milliseconds 300
}

Wait-NoTailhawk

# **A fresh process per item, and that is not caution - it is what makes a verdict mean anything.**
# One long-lived process judges every command against the state the previous one left: the first
# draft reported `Go to top` as doing nothing, because an earlier item had already put the view at
# the top, and `Reset columns` as doing nothing, because nothing had moved a column yet. Both were
# honest readings of a rigged question. Thirty-odd launches cost a couple of minutes and remove
# the whole class.
$probe = Start-Tailhawk $Log
$cal = Measure-Calibration $probe
$watch = $cal.Watch
$tolerance = $cal.Tolerance
$restless = @($TEXT_SURFACES | Where-Object { $cal.Stable -notcontains $_ })
Write-Host "watching: $($watch -join ', ')  (an idle window churns $($cal.Churn) pixels; over $tolerance counts)"
if ($restless) {
    Write-Host "not watching (changes on its own while idle): $($restless -join ', ')"
}

# The sweep is planned against one reading of the menu and then carried out against fresh ones, so
# each item is found again by **id** in the process that will run it. Matching on the label would
# be ambiguous - `Find` is the prefix of `Find next` - and ids are what the menu dispatches on.
$plan = foreach ($head in @(Read-MenuBar $probe.MainWindowHandle)) {
    foreach ($item in $head.Items) {
        if ($item.Separator) { continue }
        [pscustomobject]@{
            Head  = $head.Label
            Label = $item.Label
            Id    = $item.Id
            Popup = $item.Popup
        }
    }
}
if (-not $probe.HasExited) { $probe.Kill() }
Wait-NoTailhawk

$suspects = @()
$chosen = 0
$heading = ''


# Chooses one item, in a window of its own, and reports what it did.
#
# **It neither prints nor counts**, because a `SUSPECT` is worth asking twice before it is
# believed. The pixel surface is captured off the screen — a D3D11 swapchain cannot be rendered
# into a bitmap with `PrintWindow`, which is why `shot-window.ps1` has to pin the window topmost —
# so anything that overlaps the window for the moment of a capture makes a real change read as no
# change at all. That is not hypothetical: two consecutive runs on 2026-08-26 disagreed about
# exactly the items whose whole effect was drawn, and about nothing else.
function Invoke-Item($p, $watch, [int]$tolerance) {
    Wait-NoTailhawk
    $proc = Start-Tailhawk $Log
    try {
        $live = @(Read-MenuBar $proc.MainWindowHandle) |
            ForEach-Object { $_.Items } |
            Where-Object { $_.Id -eq $p.Id } | Select-Object -First 1
        if (-not $live) {
            return @{ Verdict = 'fail'; Detail = "id $($p.Id) is not in the menu of a fresh window"
                      Note = 'vanished' }
        }
        if (-not $live.Enabled) { return @{ Verdict = 'greyed' } }

        $before = Get-Surfaces $proc
        Send-MenuCommand $proc.MainWindowHandle $live

        # **Wait for the dialog rather than sampling once and moving on.** A file dialog can take
        # well over a second to appear, and a draft that looked once at 700 ms blamed it on
        # whichever item came next: `Export view…` was reported as raising `Open`, and
        # `Highlight rules…` as raising `Font`.
        #
        # `Wait-For` runs its condition in its own scope, so nothing assigned inside it reaches
        # here - the reading has to be taken again afterwards, which is why the condition only
        # answers yes or no. It throws on timeout, and a timeout is a real answer: the item did
        # nothing within four seconds.
        try {
            $null = Wait-For {
                $now = Get-Surfaces $proc
                $hit = @([Dlg]::Windows($proc.Id) | Where-Object { $_ -like '#32770|*' }).Count -gt 0 -or
                       (Compare-Surfaces $before $now $watch $tolerance).Count -gt 0
                Remove-Surfaces $now
                $hit
            } "$($p.Label) to do something" 4
        } catch { }
        $raised = @([Dlg]::Windows($proc.Id) | Where-Object { $_ -like '#32770|*' })

        foreach ($w in $raised) {
            $title = $w -replace '^#32770\|', ''
            if (-not (Close-Dialog $proc.Id $title)) {
                Remove-Surfaces $before
                return @{ Verdict = 'fail'; Detail = "its '$title' dialog would not close"
                          Note = 'dialog stuck' }
            }
        }
        if ($raised) { $null = Restore-Foreground $proc }
        Start-Sleep -Milliseconds 300

        if ($proc.HasExited) {
            Remove-Surfaces $before
            return @{ Verdict = 'fail'; Detail = 'the application exited'; Note = 'exited' }
        }

        $after = Get-Surfaces $proc
        $moved = Compare-Surfaces $before $after $watch $tolerance
        Remove-Surfaces $before
        Remove-Surfaces $after
        if ($raised) {
            return @{ Verdict = 'raised'
                      Detail = ($raised | ForEach-Object { $_ -replace '^#32770\|', '' }) -join ', ' }
        }
        if ($moved) { return @{ Verdict = 'moved'; Detail = ($moved -join ' and ') } }
        return @{ Verdict = 'suspect' }
    } finally {
        if (-not $proc.HasExited) { $proc.Kill() }
    }
}

foreach ($p in $plan) {
    if ($Menu -and $p.Head -notlike "$Menu*") { continue }
    if ($p.Head -ne $heading) {
        $heading = $p.Head
        Write-Host ''
        Write-Host "--- $heading ---"
    }

    if ($p.Popup) {
        Write-Host ("  {0,-26} SKIP -- a submenu; its rows are data, not commands" -f $p.Label)
        continue
    }
    if ($p.Id -ge 10100 -and $p.Id -lt 10200) {
        Write-Host ("  {0,-26} SKIP -- a recent file; opening it would change the tab set" -f $p.Label)
        continue
    }
    if ($SKIP.ContainsKey($p.Label)) {
        Write-Host ("  {0,-26} SKIP -- {1}" -f $p.Label, $SKIP[$p.Label])
        continue
    }

    $verdict = Invoke-Item $p $watch $tolerance
    if ($verdict.Verdict -ne 'greyed') { $chosen++ }

    # **A suspect is asked a second time, in another fresh window, before it is reported.** One
    # occluded capture is enough to turn a real change into "nothing happened", and a sweep that
    # cries wolf is a sweep nobody reads. Only an item that says nothing happened *twice* is
    # carried to the summary; one that moves on the retry says so, because a reading that changes
    # between two identical runs is itself worth seeing.
    if ($verdict.Verdict -eq 'suspect') {
        $again = Invoke-Item $p $watch $tolerance
        $chosen++
        if ($again.Verdict -eq 'suspect') {
            Write-Host ("  {0,-26} SUSPECT -- nothing observable changed, in two windows" -f $p.Label)
            $suspects += "$($p.Head) > $($p.Label)"
        } else {
            Write-Host ("  {0,-26} {1} {2}  (the first window read as nothing - occluded)" -f `
                $p.Label, $again.Verdict, $again.Detail)
        }
        continue
    }

    switch ($verdict.Verdict) {
        'greyed' { Write-Host ("  {0,-26} greyed" -f $p.Label) }
        'raised' { Write-Host ("  {0,-26} raised {1}" -f $p.Label, $verdict.Detail) }
        'moved' { Write-Host ("  {0,-26} moved {1}" -f $p.Label, $verdict.Detail) }
        'fail' {
            Write-Host ("  {0,-26} FAIL -- {1}" -f $p.Label, $verdict.Detail)
            $suspects += "$($p.Head) > $($p.Label) ($($verdict.Note))"
        }
    }
}

Write-Host ''
Write-Host "chose $chosen items, each in a window of its own"
if ($suspects.Count -eq 0) {
    Write-Host 'PASS: every enabled item did something observable'
} else {
    Write-Host 'SUSPECT items -- each is enabled and changed nothing that can be seen from outside:'
    $suspects | ForEach-Object { Write-Host "  $_" }
    Write-Host 'Some may be honest: an item that re-applies a state a fresh window already holds'
    Write-Host 'has nothing to change. Read them rather than trusting the count.'
    exit 1
}
