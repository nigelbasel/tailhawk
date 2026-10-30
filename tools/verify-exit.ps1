# Does the process go when its window closes?
#
# **Why this harness exists.** The owner's install was found a day after its window had closed,
# still running: one thread, no window, CPU climbing. A dump put it in the graphics driver,
# releasing the Direct3D device from a thread-local destructor during process shutdown, after
# `ExitProcess` had killed the driver's own threads (2026-10-30; `tools/read-dump.py` is what read
# it). The race cannot be forced, so this asserts the thing that can be: the process is gone within
# a few seconds of `WM_CLOSE`, with and without a UI Automation client holding its tree — the other
# documented way a Win32 process fails to exit. Needs `powershell`, not `pwsh`, and a desktop; it
# never takes the foreground. Run it with `-NoUia` as well, as `tools/check.sh` does not.
#
#     powershell -File tools/verify-exit.ps1
#     powershell -File tools/verify-exit.ps1 -NoUia
[CmdletBinding()]
param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\tailhawk.exe'),
    [string]$Log = (Join-Path (Split-Path -Parent $PSScriptRoot) 'logs\agent.log'),
    [switch]$NoUia,
    [int]$WaitSeconds = 10
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class ExitProbe {
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
'@
$proc = Start-Process $Exe -ArgumentList $Log -PassThru
$deadline = (Get-Date).AddSeconds(20)
while ((Get-Date) -lt $deadline) { $proc.Refresh(); if ($proc.MainWindowHandle -ne 0) { break }; Start-Sleep -Milliseconds 200 }
if ($proc.MainWindowHandle -eq 0) { $proc.Kill(); throw 'no window' }
Start-Sleep -Seconds 3
$hwnd = $proc.MainWindowHandle
Write-Host "pid $($proc.Id) window $hwnd  uia client: $(-not $NoUia)"
$kept = @()
if (-not $NoUia) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    $kept += $root
    Write-Host "root: '$($root.Current.Name)' type=$($root.Current.ControlType.ProgrammaticName)"
    $kids = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    Write-Host "descendants: $($kids.Count)"
    foreach ($k in $kids) { $kept += $k; $null = $k.Current.Name }
    # Patterns, as a screen reader would ask for them.
    foreach ($k in $kept) { try { $null = $k.GetSupportedPatterns() } catch {} }
}
$before = $proc.TotalProcessorTime.TotalSeconds
[void][ExitProbe]::PostMessageW($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
$gone = $proc.WaitForExit($WaitSeconds * 1000)
if ($gone) {
    Write-Host "RESULT: exited within ${WaitSeconds}s (code $($proc.ExitCode))"
} else {
    $proc.Refresh()
    Write-Host ("RESULT: STILL RUNNING after {0}s - threads={1} window={2} cpu since close={3:n1}s" -f $WaitSeconds, $proc.Threads.Count, $proc.MainWindowHandle, ($proc.TotalProcessorTime.TotalSeconds - $before))
    $proc.Threads | ForEach-Object { "  thread {0} {1} {2} cpu={3:n1}s" -f $_.Id, $_.ThreadState, $_.WaitReason, $_.TotalProcessorTime.TotalSeconds }
    Write-Host "  a dump of it, for tools/read-dump.py:  rundll32.exe comsvcs.dll, MiniDump $($proc.Id) `$env:TEMP\tailhawk-exit.dmp full"
    $proc.Kill()
    exit 1
}
