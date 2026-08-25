# V15: drives the shipped binary through UI Automation -- names, types, bounds, Value/Toggle/
# SelectionItem patterns -- and needs no foreground window, so it runs while the desktop is in use.
#
#   powershell tools/verify-uia.ps1 C:\logs\a.log C:\logs\b.log
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Log,
    [string]$SecondLog = $Log,
    [string]$Exe = ''
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]::Children
function ById($root, $id) {
    $cond = New-Object System.Windows.Automation.PropertyCondition($A::AutomationIdProperty, $id)
    $e = $root.FindFirst($Scope, $cond)
    if ($null -eq $e) { throw "no element with AutomationId '$id'" }
    $e
}
$failures = 0
function Check($name, $ok) { if ($ok) { Write-Host "  ok    $name" } else { Write-Host "  FAIL  $name"; $script:failures++ } }

if (-not $Exe) { $Exe = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "..\target\release\tailhawk.exe" }
$p = Start-Process -FilePath (Resolve-Path $Exe).Path -ArgumentList @('--stateless', '--new-instance', '--filter=e', (Resolve-Path $Log).Path, (Resolve-Path $SecondLog).Path) -PassThru
try {
    Start-Sleep -Seconds 4
    $p.Refresh()
    if ($p.MainWindowHandle -eq 0) { throw 'no main window' }
    $root = $A::FromHandle($p.MainWindowHandle)
    Check 'root is a Pane named Tailhawk' ($root.Current.Name -eq 'Tailhawk' -and $root.Current.ControlType.ProgrammaticName -eq 'ControlType.Pane')
    $kids = $root.FindAll($Scope, [System.Windows.Automation.Condition]::TrueCondition)
    Write-Host "  children: $($kids.Count)"
    foreach ($e in $kids) { Write-Host ("        {0,-10} | {1,-36} | {2,-22} | {3}" -f $e.Current.AutomationId, $e.Current.Name, $e.Current.ControlType.ProgrammaticName, $e.Current.BoundingRectangle) }
    Check 'every child has a name, a type and a rectangle' (($kids | Where-Object { -not $_.Current.Name -or $_.Current.BoundingRectangle.IsEmpty }).Count -eq 0)

    # There is no in-window search field to drive any more: Ctrl+F is the classic Find dialog,
    # which is a native window with Windows' own UIA — verify-find.ps1 covers it behaviourally.
    $status = ById $root 'status'
    $sv = $status.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    Check 'the status bar carries the status text' ($sv.Current.Value -match 'lines')

    $chip = ById $root 'chip-0'
    Check 'the chip is a Button named for its text' ($chip.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and $chip.Current.Name -eq 'include e')
    $tp = $chip.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
    $before = $tp.Current.ToggleState
    $tp.Toggle()
    Start-Sleep -Milliseconds 500
    Check 'Toggle flips the chip' ($tp.Current.ToggleState -ne $before)
    $ip = $chip.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $ip.Invoke()
    Start-Sleep -Milliseconds 500
    $gone = $null -eq $root.FindFirst($Scope, (New-Object System.Windows.Automation.PropertyCondition($A::AutomationIdProperty, 'chip-0')))
    Check 'Invoke removes the chip' $gone

    if ($SecondLog -ne $Log) {
        $tab0 = ById $root 'tab-0'
        $sp = $tab0.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
        $sp.Select()
        Start-Sleep -Milliseconds 500
        Check 'Select shows the first tab' ($sp.Current.IsSelected)
        $p.Refresh()
        Check 'the title follows the tab' ($p.MainWindowTitle -match [regex]::Escape((Split-Path $Log -Leaf)))
    }
} finally {
    Get-Process tailhawk -ErrorAction SilentlyContinue | Where-Object { $_.Id -eq $p.Id } | Stop-Process -Force
}
if ($failures -gt 0) { Write-Host "$failures check(s) failed"; exit 1 }
Write-Host 'all UIA checks passed'
