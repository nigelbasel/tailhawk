# Drives the filter panel from the keyboard in both themes, and photographs it.
#
# **Why a harness.** The panel's decisions are pure and tested in `filterpanel.rs` and `main.rs` —
# its rows, its band, which Edit menu items a selection enables. What no test here can answer is
# whether the list view does what those decisions ask: that F6 reaches it, that Enter and Delete in
# it run Edit > Edit filter… and Remove filter, that Esc gives the keyboard back, and that its rows
# are whole and its ground dark in the dark theme. The first native build had all of those to find.
#
#   powershell tools/verify-panel.ps1 -Log C:\path\to\some.log
#
# Needs `powershell`, not `pwsh`, and a desktop session: it sends keystrokes and takes the
# foreground for as long as it runs. The screenshots land beside `-Shots`.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Log,
    [string]$Shots = $env:TEMP
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Dialog.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$exe = Get-TailhawkExe
$fails = 0
function Check($name, $ok, $saying) {
    if ($ok) { Write-Host "  ok    $name  $saying" } else { Write-Host "  FAIL  $name  $saying"; $script:fails++ }
}
# The focused element's type, name and window class — the class is what tells the list view from
# the log, because the managed client names neither.
function Focused {
    $f = $A::FocusedElement
    "$($f.Current.ControlType.ProgrammaticName)|$($f.Current.Name)|$($f.Current.ClassName)"
}

foreach ($theme in 'dark', 'light') {
    Write-Host "== $theme"
    $p = Start-Process -FilePath $exe -ArgumentList @('--stateless', '--new-instance', "--theme=$theme", '--filter=e', '--exclude=DEBUG', (Resolve-Path $Log).Path) -PassThru
    try {
        $null = Wait-For { $p.Refresh(); $p.MainWindowHandle -ne 0 -and (Get-StatusText $p) -match 'lines' } 'the window' 20
        Start-Sleep -Milliseconds 1200
        $wsh = New-Object -ComObject WScript.Shell
        [void]$wsh.AppActivate($p.Id)
        Start-Sleep -Milliseconds 400
        $bmp = [Shot]::Client($p.MainWindowHandle)
        $shot = Join-Path $Shots "tailhawk-verify-panel-$theme.png"
        $bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        Write-Host "  shot  $shot"

        $bar = Read-MenuBar $p.MainWindowHandle
        Check 'Edit filter is greyed with nothing selected' (-not (Find-MenuItem $bar 'Edit' 'Edit filter').Enabled) ''

        $wsh.SendKeys('{F6}')
        Start-Sleep -Milliseconds 500
        $f = Focused
        Check 'F6 moves the keyboard into the list' ($f -match 'SysListView32') $f
        $wsh.SendKeys('{DOWN}')
        Start-Sleep -Milliseconds 400
        $bar = Read-MenuBar $p.MainWindowHandle
        Check 'a selected row enables Edit > Edit filter' (Find-MenuItem $bar 'Edit' 'Edit filter').Enabled ''
        Check 'and Edit > Remove filter' (Find-MenuItem $bar 'Edit' 'Remove filter').Enabled ''

        $wsh.SendKeys('{ENTER}')
        Start-Sleep -Milliseconds 1200
        $dlg = Get-Dialog $p.Id 'Edit Filter'
        if ($dlg -eq [IntPtr]::Zero) { $dlg = Get-Dialog $p.Id '' }
        Check 'Enter in the list opens the Filter dialog' ($dlg -ne [IntPtr]::Zero) "hwnd=$dlg"
        if ($dlg -ne [IntPtr]::Zero) {
            [void][Dlg]::PostMessageW($dlg, [Dlg]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 600
        }

        [void]$wsh.AppActivate($p.Id)
        Start-Sleep -Milliseconds 300
        $wsh.SendKeys('{F6}')
        Start-Sleep -Milliseconds 400
        $before = Get-StatusText $p
        $wsh.SendKeys('{DELETE}')
        Start-Sleep -Milliseconds 800
        $after = Get-StatusText $p
        Check 'Delete in the list removes a filter' ($before -ne $after) "now: $after"

        $wsh.SendKeys('{ESC}')
        Start-Sleep -Milliseconds 400
        $f = Focused
        Check 'Esc returns the keyboard to the log' ($f -match 'TailhawkMain') $f
        Check 'still running' (-not $p.HasExited) ''
    }
    finally {
        if (-not $p.HasExited) { $p.Kill() }
    }
}
Write-Host ''
Write-Host "verify-panel: $fails failed"
if ($fails) { exit 1 }
