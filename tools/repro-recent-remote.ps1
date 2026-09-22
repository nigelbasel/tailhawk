# Reproduces the Open Recent > remote crash without a live Loki and without touching
# Nigel's installed settings. Plants a fixture beside the dev binary (backed up and
# restored), starts it, posts the WM_COMMAND a menu click posts, and reports whether
# the process is still alive.
#
# The source points at 127.0.0.1:9 (discard). The panic happens before any socket
# matters, so nothing reaches a real server either way.

$ErrorActionPreference = 'Stop'

$exe      = 'C:\dev\git\TailHawk\target\release\tailhawk.exe'
$settings = 'C:\dev\git\TailHawk\target\release\tailhawk.settings.toml'
$backup   = "$settings.repro-backup"

if (Test-Path $backup) { Remove-Item $backup -Force }
if (Test-Path $settings) { Copy-Item $settings $backup }

$fixture = @'
# Fixture for the Open Recent > remote repro. Restored after the run.

[appearance]
theme = "dark"
font_size = 16

[window]
x = 90
y = 90
width = 1100
height = 300
maximized = false

[recent]
files = ["loki://repro?apps=Worker"]

[[source]]
name = "repro"
url = "http://127.0.0.1:9/loki"
token_url = ""
client_id = ""
scope = ""
query = "{environment=\"dev\"}"
'@
# WriteAllText with a BOM-less encoding, not Set-Content: PowerShell 5.1's -Encoding utf8
# writes a BOM, and the first run of this repro loaded nothing because of it.
[System.IO.File]::WriteAllText($settings, $fixture, (New-Object System.Text.UTF8Encoding($false)))

# A panic in a window procedure never reaches a console - Windows kills the process on
# STATUS_FATAL_USER_CALLBACK_EXCEPTION first. This is the app's own sink for it.
$panicLog = Join-Path $env:TEMP 'tailhawk-repro-panic.log'
if (Test-Path $panicLog) { Remove-Item $panicLog -Force }
$env:TAILHAWK_PANIC_LOG = $panicLog

if (-not ([System.Management.Automation.PSTypeName]'Win32Repro').Type) {
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Win32Repro {
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool PostMessageW(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr hWnd, StringBuilder name, int n);
}
'@
}

$proc = Start-Process -FilePath $exe -PassThru

# The process's own main window, not FindWindowW: PowerShell marshals a $null string
# argument as an empty string, so FindWindowW('TailhawkMain', $null) never matches.
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 100
    $proc.Refresh()
    if ($proc.HasExited) { break }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero) { $hwnd = $proc.MainWindowHandle; break }
}

if ($hwnd -ne [IntPtr]::Zero) {
    $sb = New-Object System.Text.StringBuilder 256
    [void][Win32Repro]::GetClassNameW($hwnd, $sb, 256)
    Write-Output "window: $hwnd class $($sb.ToString())"
}

if ($hwnd -eq [IntPtr]::Zero) {
    Write-Output 'RESULT: no window appeared - the run tells us nothing'
} else {
    Start-Sleep -Milliseconds 1200
    # WM_COMMAND, id 10100 = menubar::ID_RECENT_BASE, lparam 0 = "came from the menu".
    $null = [Win32Repro]::PostMessageW($hwnd, 0x0111, [IntPtr]10100, [IntPtr]::Zero)
    $died = $false
    for ($t = 1; $t -le 10; $t++) {
        Start-Sleep -Milliseconds 1000
        $proc.Refresh()
        if ($proc.HasExited) {
            Write-Output "RESULT: CRASHED after ~${t}s - exit code $($proc.ExitCode)"
            $died = $true
            break
        }
    }
    if (-not $died) {
        Write-Output 'RESULT: ALIVE - the window survived 10s after choosing the recent remote'
    }
}

if (Test-Path $panicLog) {
    Write-Output '--- panic log ---'
    Get-Content $panicLog | ForEach-Object { Write-Output $_ }
} else {
    Write-Output '(no panic was recorded)'
}

if (-not $proc.HasExited) { $proc.Kill(); [void]$proc.WaitForExit(5000) }
Start-Sleep -Milliseconds 400
if (Test-Path $backup) {
    Move-Item $backup $settings -Force
} else {
    Remove-Item $settings -Force -ErrorAction SilentlyContinue
}
Write-Output 'settings restored'
