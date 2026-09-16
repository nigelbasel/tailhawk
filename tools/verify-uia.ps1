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
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
if (-not ('PanelList' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PanelList {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public static IntPtr Find(IntPtr main) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(main, (h, l) => {
            StringBuilder c = new StringBuilder(64); GetClassNameW(h, c, 64);
            if (c.ToString() == "SysListView32") { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
}
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

    # **The filter panel is Windows controls since 2026-09-15**, so it is not our provider's to
    # describe, and this script's managed UIA client reports Windows' own children as unnamed panes.
    # So the list is asked directly, with messages that carry no pointer and so answer across
    # processes: one row for `--filter=e`, its check box ticked. The commands that act on a row are
    # on the Edit menu and must be greyed until one is selected. The keyboard's path through the
    # panel — F6, Enter, Delete, Esc — needs the foreground and is `verify-panel.ps1`'s.
    $LVM_GETITEMCOUNT = 0x1004
    $LVM_GETITEMSTATE = 0x102C
    $list = [PanelList]::Find($p.MainWindowHandle)
    Check 'the filter panel is a real list view' ($list -ne [IntPtr]::Zero)
    if ($list -ne [IntPtr]::Zero) {
        $rows = [PanelList]::SendMessageW($list, $LVM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        Check 'it lists the one filter' ($rows -eq 1)
        $state = [PanelList]::SendMessageW($list, $LVM_GETITEMSTATE, [IntPtr]::Zero, [IntPtr]0xF000).ToInt32()
        Check 'its check box is ticked' (($state -band 0xF000) -eq 0x2000)
    }
    $bar = Read-MenuBar $p.MainWindowHandle
    Check 'Edit > Edit filter is greyed with nothing selected' (-not (Find-MenuItem $bar 'Edit' 'Edit filter').Enabled)
    Check 'Edit > Remove filter is greyed with nothing selected' (-not (Find-MenuItem $bar 'Edit' 'Remove filter').Enabled)

    # **The log's text, readable by a screen reader** — `SPEC.md` §14.1's grid text provider. Each
    # shown pane is a Document with the Text pattern. The checks agree with each other rather than
    # with the file, because a recognised format reads as its aligned columns, not as its raw lines:
    # the caret's line is one line, a line move moves, the visible lines have rectangles inside the
    # window, and the point in the middle of the first of them reads back as that same line.
    $TU = [System.Windows.Automation.Text.TextUnit]
    $grid = ById $root 'grid-0'
    Check 'the log is a Document' ($grid.Current.ControlType.ProgrammaticName -eq 'ControlType.Document')
    $text = $grid.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
    $whole = $text.DocumentRange.GetText(4000)
    Check 'its document range reads the log' ($whole.Length -gt 20 -and $whole.Contains("`n"))
    $selection = $text.GetSelection()
    Check 'the caret is a range of its own' ($selection.Count -eq 1)
    if ($selection.Count -ge 1) {
        $caret = $selection[0].Clone()
        $caret.ExpandToEnclosingUnit($TU::Line)
        $line = $caret.GetText(-1)
        Check 'the caret expands to one line' ($line.Length -gt 0 -and $line.TrimEnd("`n").IndexOf("`n") -lt 0)
        Check 'a line move reports moving' ($caret.Move($TU::Line, 1) -eq 1)
    }
    $visible = $text.GetVisibleRanges()
    $rects = $visible[0].GetBoundingRectangles()
    $window = $root.Current.BoundingRectangle
    Check 'the visible lines have rectangles inside the window' ($rects.Count -gt 1 -and ($rects | Where-Object { -not $window.Contains($_.TopLeft) }).Count -eq 0)
    if ($rects.Count -gt 0) {
        $first = $rects[0]
        $middle = New-Object System.Windows.Point(($first.Left + 4), ($first.Top + $first.Height / 2))
        $hit = $text.RangeFromPoint($middle)
        $hit.ExpandToEnclosingUnit($TU::Line)
        $top = $visible[0].Clone()
        $top.ExpandToEnclosingUnit($TU::Line)
        Check 'a point on the first line reads back as that line' ($hit.GetText(-1) -eq $top.GetText(-1))
    }

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
