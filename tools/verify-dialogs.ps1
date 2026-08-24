# Proves the menu dialogs are native windows, not overlays drawn on the grid.
#
# The owner's report, 2026-08-24: About "just dumps the text on top of the window", and every Help
# item behaved like a dropdown. The fix routes About through TaskDialogIndirect and Font… through
# ChooseFontW -- and the one thing an overlay can never do is put a second top-level window of the
# dialog class `#32770` on the desktop. So that is what this checks: click the menu item, wait for
# a real `#32770` window with the right title, close it, and confirm the app survived.
#
#   powershell tools/verify-dialogs.ps1
#
# Geometry comes from the product via TAILHAWK_DUMP_MENU_HITS, exactly as verify-menus.ps1 does.
[CmdletBinding()]
param(
    [string]$Log = "$env:TEMP\tailhawk-verify-dialogs.log",
    [string]$Hits = "$env:TEMP\tailhawk-menu-hits.txt"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$src = Get-Content (Join-Path $PSScriptRoot 'verify-touch.ps1') -Raw
Add-Type ([regex]::Match($src, "(?s)Add-Type @'\r?\n(.*?)\r?\n'@").Groups[1].Value)
if (-not [Touch]::InitializeTouchInjection(1, 3)) {
    throw 'InitializeTouchInjection failed'
}

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Dlg {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
}
'@

$env:TAILHAWK_DUMP_MENU_HITS = $Hits

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

function Wait-NoTailhawk {
    $null = Wait-For {
        (Get-Process tailhawk -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0
    } 'the previous instance to exit' 15
    Start-Sleep -Milliseconds 300
}

function Tap([int]$x, [int]$y) {
    [Touch]::Send($x, $y, [Touch]::DOWN)
    Start-Sleep -Milliseconds 60
    [Touch]::Send($x, $y, [Touch]::UP)
    Start-Sleep -Milliseconds 450
}

if (-not (Test-Path $Log)) {
    $sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
    for ($i = 0; $i -lt 200; $i++) {
        $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
    }
    $sw.Close()
}

# Opens menu $menuIndex and clicks entry $row (negative counts from the end), then waits for a
# `#32770` window titled $title. Returns $true when it appeared and the app survived it closing.
function Test-Dialog([int]$menuIndex, [int]$row, [string]$title, [string]$name) {
    Wait-NoTailhawk
    $proc = Start-Tailhawk $Log
    try {
        $po = New-Object Shot+POINT
        [void][Shot]::ClientToScreen($proc.MainWindowHandle, [ref]$po)
        $head = Read-Hits 'heading' | Where-Object { $_.Index -eq $menuIndex }
        if (-not $head) { Write-Host "$name : FAIL -- no heading rect"; return $false }
        Tap ($po.X + $head.CX) ($po.Y + $head.CY)
        $entries = @(Read-Hits 'entry')
        if (-not $entries) { Write-Host "$name : FAIL -- menu drew no entries"; return $false }
        $at = if ($row -lt 0) { $entries.Count + $row } else { $row }
        $entry = $entries | Where-Object { $_.Index -eq $at }
        if (-not $entry) { Write-Host "$name : FAIL -- no entry $at"; return $false }
        Tap ($po.X + $entry.CX) ($po.Y + $entry.CY)

        $dlg = [IntPtr]::Zero
        $null = Wait-For {
            $script:dlg = [Dlg]::FindWindowW('#32770', $title)
            $script:dlg -ne [IntPtr]::Zero
        } "the $name dialog" 10
        $dlg = $script:dlg
        Write-Host "$name : native dialog '#32770' titled '$title' is on screen"

        # WM_CLOSE: the About dialog allows cancellation and ChooseFont reads it as Cancel.
        [void][Dlg]::PostMessageW($dlg, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 600
        if ([Dlg]::FindWindowW('#32770', $title) -ne [IntPtr]::Zero) {
            Write-Host "$name : FAIL -- the dialog did not close"
            return $false
        }
        if ($proc.HasExited) {
            Write-Host "$name : FAIL -- closing the dialog took the app with it"
            return $false
        }
        Write-Host "$name : closed cleanly, app still running"
        return $true
    } finally {
        if (-not $proc.HasExited) { $proc.Kill() }
    }
}

$ok = $true
# Help is menu 6: Keyboard map is row 1, About Tailhawk the last row. Format is menu 3 with
# Font... last; Settings is menu 5 with Preferences... last.
$ok = (Test-Dialog 6 -1 'About Tailhawk' 'Help > About') -and $ok
$ok = (Test-Dialog 3 -1 'Font' 'Format > Font...') -and $ok
$ok = (Test-Dialog 6 1 'Keyboard map' 'Help > Keyboard map') -and $ok
$ok = (Test-Dialog 5 -1 'Preferences' 'Settings > Preferences...') -and $ok

if ($ok) { Write-Host 'PASS: all four menu dialogs are real native dialogs' }
else { Write-Host 'FAIL'; exit 1 }
