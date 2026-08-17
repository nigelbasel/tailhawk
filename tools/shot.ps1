# Opens the shipped binary on a file, waits a moment, and saves a screenshot of the client area --
# the quickest way to *look* at a change on real content, which this project's rules require.
#
#   powershell tools/shot.ps1 logs\agent.log
#   powershell tools/shot.ps1 C:\logs\app.log -Keys '^l' 'error' '{ENTER}' -Shot out.png
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Log,
    [string[]]$Keys = @(),
    [string]$Shot = "$env:TEMP\tailhawk-shot.png",
    [int]$SettleMs = 1200,
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

$proc = Start-Tailhawk (Resolve-Path $Log).Path
try {
    $wsh = New-Object -ComObject WScript.Shell
    foreach ($k in $Keys) { $wsh.SendKeys($k); Start-Sleep -Milliseconds 250 }
    Start-Sleep -Milliseconds $SettleMs
    $proc.Refresh()
    Write-Host "title: $($proc.MainWindowTitle)"
    $bmp = [Shot]::Client($proc.MainWindowHandle)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "screenshot: $Shot"
}
finally {
    if (-not $KeepWindow -and -not $proc.HasExited) { $proc.Kill() }
}
