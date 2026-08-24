# Clicks every item in every menu and reports what each one did.
#
# `File ▸ Open…` did nothing when **clicked** for a month while working perfectly from the keyboard
# and the palette. No test caught it, and no test could have: the tests and the keyboard go through
# the same `Shell::run`, and what was missing was in the mouse path after it. The only thing that
# finds a bug of that shape is clicking the thing.
#
# What it can decide on its own is deliberately modest -- it reports **observable effect**, not
# correctness. An enabled item that changes nothing at all is the signature of the Open bug and is
# flagged; whether an item did the *right* thing is a human's judgement from the log it prints.
#
#   powershell tools/verify-menus.ps1
#   powershell tools/verify-menus.ps1 -Menu File
#
# **The geometry comes from the product, not from arithmetic.** `TAILHAWK_DUMP_MENU_HITS` makes the
# bar write out every rect it drew, and this clicks the centre of each. An earlier draft assumed a
# uniform row pitch and drifted, because a separator is drawn half the height of an item: it clicked
# `Exit` while reporting that it had clicked a separator. A sweep that clicks the wrong thing is
# worse than no sweep.
[CmdletBinding()]
param(
    [string]$Menu = '',
    [string]$Log = "$env:TEMP\tailhawk-verify-menus.log",
    [string]$Hits = "$env:TEMP\tailhawk-menu-hits.txt"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$src = Get-Content (Join-Path $PSScriptRoot 'verify-touch.ps1') -Raw
Add-Type ([regex]::Match($src, "(?s)Add-Type @'\r?\n(.*?)\r?\n'@").Groups[1].Value)
if (-not [Touch]::InitializeTouchInjection(1, 3)) {
    throw 'InitializeTouchInjection failed'
}

$MENU_NAMES = @('File', 'Edit', 'View', 'Format', 'Rules', 'Settings', 'Help')
$env:TAILHAWK_DUMP_MENU_HITS = $Hits

# The rects the bar last drew, as `kind index x0 y0 x1 y1`. Rewritten every frame, so it describes
# whatever is on screen at the moment it is read.
function Read-Hits([string]$kind) {
    if (-not (Test-Path $Hits)) { return @() }
    Get-Content $Hits | ForEach-Object {
        $f = $_ -split ' '
        if ($f[0] -eq $kind) {
            [pscustomobject]@{
                Index = [int]$f[1]
                CX    = [int](([double]$f[2] + [double]$f[4]) / 2)
                CY    = [int](([double]$f[3] + [double]$f[5]) / 2)
            }
        }
    }
}

if (-not (Test-Path $Log)) {
    $sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
    for ($i = 0; $i -lt 5000; $i++) {
        $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
    }
    $sw.Close()
}

function Tap([int]$x, [int]$y) {
    [Touch]::Send($x, $y, [Touch]::DOWN)
    Start-Sleep -Milliseconds 60
    [Touch]::Send($x, $y, [Touch]::UP)
    Start-Sleep -Milliseconds 450
}

# A fingerprint of the rows only -- the status bar carries live frame timings and would report a
# change on every capture. Same reason `verify-touch.ps1` excludes it.
function Get-Frame([IntPtr]$hwnd) {
    $bmp = [Shot]::Client($hwnd)
    try {
        $sb = New-Object System.Text.StringBuilder
        for ($y = 100; $y -lt $bmp.Height - 40; $y += 9) {
            for ($x = 0; $x -lt $bmp.Width; $x += 13) {
                [void]$sb.Append($bmp.GetPixel($x, $y).ToArgb().ToString('x'))
            }
        }
        $md5 = [System.Security.Cryptography.MD5]::Create()
        [BitConverter]::ToString($md5.ComputeHash([Text.Encoding]::ASCII.GetBytes($sb.ToString())))
    } finally { $bmp.Dispose() }
}

$findings = @()

for ($menuIndex = 0; $menuIndex -lt $MENU_NAMES.Count; $menuIndex++) {
    $name = $MENU_NAMES[$menuIndex]
    if ($Menu -and $name -ne $Menu) { continue }

    # How many items this menu has, learned by opening it once.
    $probe = Start-Tailhawk $Log
    $po = New-Object Shot+POINT
    [void][Shot]::ClientToScreen($probe.MainWindowHandle, [ref]$po)
    $head = Read-Hits 'heading' | Where-Object { $_.Index -eq $menuIndex }
    if (-not $head) {
        Write-Host "$name : no heading rect -- is the bar drawn?"
        if (-not $probe.HasExited) { $probe.Kill() }
        continue
    }
    Tap ($po.X + $head.CX) ($po.Y + $head.CY)
    $count = (Read-Hits 'entry' | Measure-Object).Count
    if (-not $probe.HasExited) { $probe.Kill() }
    Start-Sleep -Milliseconds 250
    Write-Host "$name ($count items)"

    for ($row = 0; $row -lt $count; $row++) {

        # **A fresh process per item.** An item that opens a modal, closes the tab or changes the
        # theme leaves the window in a state the next item would be measured against; restarting is
        # the only way each reading means what it says.
        $proc = Start-Tailhawk $Log
        $hwnd = $proc.MainWindowHandle
        $o = New-Object Shot+POINT
        [void][Shot]::ClientToScreen($hwnd, [ref]$o)

        try {
            $before = Get-Frame $hwnd
            $title0 = $proc.MainWindowTitle

            Tap ($o.X + $head.CX) ($o.Y + $head.CY)
            # The rects for the list that is now open -- read after opening it, because the dump
            # describes what is on screen and before the click there was no list.
            $entry = Read-Hits 'entry' | Where-Object { $_.Index -eq $row }
            if (-not $entry) {
                $findings += [pscustomobject]@{ Menu = $name; Row = $row; Result = 'no rect' }
                continue
            }
            Tap ($o.X + $entry.CX) ($o.Y + $entry.CY)
            Start-Sleep -Milliseconds 500

            $proc.Refresh()
            $fg = [Shot]::GetForegroundWindow()
            $result = if ($proc.HasExited) {
                'process exited'
            } elseif ($fg -ne $hwnd -and $fg -ne 0) {
                'dialog opened'
            } elseif ($proc.MainWindowTitle -ne $title0) {
                'title changed'
            } elseif ((Get-Frame $hwnd) -ne $before) {
                'screen changed'
            } else {
                'NOTHING'
            }
            $findings += [pscustomobject]@{ Menu = $name; Row = $row; Result = $result }
            Write-Host ("{0,-9} row {1,2}  {2}" -f $name, $row, $result)
        } finally {
            if (-not $proc.HasExited) { $proc.Kill() }
            Start-Sleep -Milliseconds 250
        }
    }
}

Write-Host ''
Write-Host 'Items that produced no observable effect at all:'
$dead = $findings | Where-Object { $_.Result -eq 'NOTHING' }
if (-not $dead) {
    Write-Host '  none'
} else {
    $dead | ForEach-Object { Write-Host ("  {0} row {1}" -f $_.Menu, $_.Row) }
}
Write-Host ''
Write-Host 'A disabled item and a past-the-end row both read as NOTHING -- check each against the'
Write-Host 'menu before calling it a defect.'
