# Proves File > Open Recent end to end: a file opened lands in the settings' [recent] list, the
# submenu offers it on the next launch, and clicking the entry opens the file.
#
#   powershell tools/verify-recent.ps1
#
# Geometry from TAILHAWK_DUMP_MENU_HITS, as verify-menus.ps1. Fresh processes per phase, because
# what is being tested is precisely what survives between them.
[CmdletBinding()]
param(
    [string]$Log = "$env:TEMP\tailhawk-verify-recent.log",
    [string]$Hits = "$env:TEMP\tailhawk-menu-hits.txt"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$src = Get-Content (Join-Path $PSScriptRoot 'verify-touch.ps1') -Raw
Add-Type ([regex]::Match($src, "(?s)Add-Type @'\r?\n(.*?)\r?\n'@").Groups[1].Value)
if (-not [Touch]::InitializeTouchInjection(1, 3)) {
    throw 'InitializeTouchInjection failed'
}

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

$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt 100; $i++) {
    $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
}
$sw.Close()

# Phase 1: open the file, let it land, close -- the list is written on successful open.
Wait-NoTailhawk
$proc = Start-Tailhawk $Log
Start-Sleep -Milliseconds 800
if (-not $proc.HasExited) { $proc.Kill() }

$exe = Get-TailhawkExe
$settings = Join-Path (Split-Path $exe) 'tailhawk.settings.toml'
$toml = Get-Content $settings -Raw
if ($toml -notmatch '\[recent\]') { Write-Host 'FAIL: no [recent] section was written'; exit 1 }
if ($toml -notmatch [regex]::Escape('tailhawk-verify-recent.log')) {
    Write-Host 'FAIL: the opened file is not in the recent list'; exit 1
}
Write-Host 'opened file landed in [recent]'

# Phase 2: launch with no file at all -- the welcome screen -- and open it back through the menu.
Wait-NoTailhawk
$proc = Start-Process $exe -PassThru
$null = Wait-For { $proc.Refresh(); $proc.MainWindowHandle -ne [IntPtr]::Zero } 'a window' 15
Start-Sleep -Milliseconds 900
try {
    $po = New-Object Shot+POINT
    [void][Shot]::ClientToScreen($proc.MainWindowHandle, [ref]$po)
    $file = Read-Hits 'heading' | Where-Object { $_.Index -eq 0 }
    if (-not $file) { Write-Host 'FAIL: no File heading'; exit 1 }
    Tap ($po.X + $file.CX) ($po.Y + $file.CY)
    $entries = @(Read-Hits 'entry')
    # Open Recent sits directly under Open...
    Tap ($po.X + $entries[1].CX) ($po.Y + $entries[1].CY)
    $inner = @(Read-Hits 'entry')
    if (-not $inner) { Write-Host 'FAIL: the submenu did not open'; exit 1 }
    Write-Host "Open Recent opened with $($inner.Count) rows"
    Tap ($po.X + $inner[0].CX) ($po.Y + $inner[0].CY)
    $null = Wait-For {
        $proc.Refresh(); $proc.MainWindowTitle -match 'tailhawk-verify-recent\.log'
    } 'the recent file to open' 10
    Write-Host 'PASS: the recent entry opened the file from the welcome screen'
} finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}
