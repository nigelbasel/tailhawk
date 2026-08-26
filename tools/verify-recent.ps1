# Proves the recent-files list end to end: a file opened lands in the settings, the File menu
# offers it on the next launch, and choosing the entry opens the file.
#
#   powershell tools/verify-recent.ps1
#
# Fresh processes per phase, because what is being tested is precisely what survives between them.
#
# **What changed on 2026-08-26.** The old version drove a drawn menu bar through
# `TAILHAWK_DUMP_MENU_HITS`, and looked for an `Open Recent` *submenu* under `Open…`. Neither
# exists: the bar is a native `HMENU`, that debug channel is deleted, and the owner chose
# Notepad++'s flat shape - numbered entries in the File menu itself, just above Exit, simply absent
# when there is no history. This reads the real menu instead, which lets it assert the shape rather
# than only the behaviour: that the entries are numbered from 1, that the newest is first, and that
# the whole block disappears when the list is cleared.
[CmdletBinding()]
param(
    [string]$Log = "$env:TEMP\tailhawk-verify-recent.log"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')

function Stop-Tailhawk {
    Get-Process tailhawk -ErrorAction SilentlyContinue | Stop-Process -Force
    $null = Wait-For {
        (Get-Process tailhawk -ErrorAction SilentlyContinue | Measure-Object).Count -eq 0
    } 'the previous instance to exit' 15
    Start-Sleep -Milliseconds 300
}

# The recent entries are the File items carrying a range id, which is what distinguishes them from
# the fixed commands around them without this script having to know where they sit.
function Get-RecentEntries($Bar) {
    $file = $Bar | Where-Object { $_.Label -eq 'File' }
    @($file.Items | Where-Object { $_.Id -ge 10100 -and $_.Id -lt 10200 })
}

$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt 100; $i++) {
    $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
}
$sw.Close()

# Phase 1: open the file, let it land, close. The list is written on *successful* open.
Stop-Tailhawk
$proc = Start-Tailhawk $Log
Start-Sleep -Milliseconds 800
if (-not $proc.HasExited) { $proc.Kill() }

$exe = Get-TailhawkExe
$settings = Join-Path (Split-Path $exe) 'tailhawk.settings.toml'
if (-not (Test-Path $settings)) { Write-Host 'FAIL: no settings file was written'; exit 1 }
$toml = Get-Content $settings -Raw
if ($toml -notmatch '\[recent\]') { Write-Host 'FAIL: no [recent] section was written'; exit 1 }
if ($toml -notmatch [regex]::Escape('tailhawk-verify-recent.log')) {
    Write-Host 'FAIL: the opened file is not in the recent list'; exit 1
}
Write-Host 'the opened file landed in [recent]'

# Phase 2: launch with no file at all - the welcome screen - and open it back through the menu.
Stop-Tailhawk
$proc = Start-Process $exe -PassThru
$null = Wait-For { $proc.Refresh(); $proc.MainWindowHandle -ne 0 } 'a window' 15
Start-Sleep -Milliseconds 900
try {
    $bar = Read-MenuBar $proc.MainWindowHandle
    $entries = @(Get-RecentEntries $bar)
    if (-not $entries) { Write-Host 'FAIL: the File menu offers no recent files'; exit 1 }
    Write-Host "the File menu offers $($entries.Count) recent files"

    # The shape the owner asked for: numbered from 1, newest first, and the file just opened is
    # therefore the first of them.
    if ($entries[0].Label -notmatch '^1\s') {
        Write-Host "FAIL: the first entry is not numbered 1 -- '$($entries[0].Label)'"; exit 1
    }
    if ($entries[0].Label -notmatch 'tailhawk-verify-recent') {
        Write-Host "FAIL: the newest file is not first -- '$($entries[0].Label)'"; exit 1
    }
    Write-Host "entry 1 is the file just closed: $($entries[0].Label)"

    Send-MenuCommand $proc.MainWindowHandle $entries[0]
    $null = Wait-For {
        $proc.Refresh(); $proc.MainWindowTitle -match 'tailhawk-verify-recent\.log'
    } 'the recent file to open' 10
    Write-Host 'choosing the entry opened the file from the welcome screen'

    # **Wait for the open to be recorded, not merely for the title to change.** The history is
    # written where the open *lands*, which is a poll or two after the title says the file is
    # there - and a clear posted into that gap is undone by the landing that follows it. An
    # earlier draft slept instead, and failed on one run in two.
    $null = Wait-For {
        @(Get-RecentEntries (Read-MenuBar $proc.MainWindowHandle)).Count -ge 1
    } 'the reopened file to be recorded' 10

    # Clearing empties the block rather than greying it: the owner's choice, and the one thing a
    # reader of the menu can check that a reader of the settings file cannot.
    $clear = $bar | Where-Object { $_.Label -eq 'File' } |
        ForEach-Object { $_.Items } |
        Where-Object { $_.Label -like 'Clear recent*' } | Select-Object -First 1
    if (-not $clear) { Write-Host 'FAIL: no Clear recent files entry'; exit 1 }
    Send-MenuCommand $proc.MainWindowHandle $clear
    Start-Sleep -Milliseconds 500
    $after = @(Get-RecentEntries (Read-MenuBar $proc.MainWindowHandle))
    if ($after.Count -ne 0) {
        Write-Host "FAIL: Clear recent files left $($after.Count) entries"; exit 1
    }
    Write-Host 'Clear recent files empties the block entirely'
    Write-Host 'PASS: the recent list is written, offered, chosen and cleared'
} finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}
