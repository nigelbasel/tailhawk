# Drives the surfaces the 2026-09-09 UX review changed, and reports what each one did.
#
# **Why a harness rather than a unit test.** Every decision behind these surfaces is already pinned
# by a pure test — which buttons the toolbar has, what the chooser's list says, which key runs the
# rules editor. What no test in this repository can answer is whether the *control* does what the
# decision asked for: a dropdown button that never drops a menu, a dialog whose template Windows
# refuses (`DialogBoxIndirectParamW` returns and nothing appears), a menu id nothing dispatches.
# `verify-menus.ps1` exists because `File > Open…` did nothing for a month while its tests passed.
#
#   powershell tools/verify-ux.ps1 -Log C:\path\to\some.log
#
# Needs `powershell`, not `pwsh`, and a desktop session: the dialogs are real windows and two of
# the checks send keystrokes. It takes the foreground for as long as it runs.

[CmdletBinding()]
param(
    [string]$Log = '',
    [int]$Seconds = 12
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Dialog.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')

if (-not ('Ctl' -as [type])) {
    Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class Ctl {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);

    // Every descendant window of `parent`, as "class|id|handle". The toolbar, its rebar, the tab
    // strip, the header controls and the status bar all come back from one call.
    public static string[] Children(IntPtr parent) {
        List<string> found = new List<string>();
        EnumChildWindows(parent, (h, l) => {
            StringBuilder c = new StringBuilder(64); GetClassNameW(h, c, 64);
            found.Add(c.ToString() + "|" + GetDlgCtrlID(h) + "|" + h.ToInt64());
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
}
'@
}

# The toolbar's own messages, and the two bits this script asks about.
$TB_BUTTONCOUNT = 0x0418
$TB_GETBUTTON = 0x0417       # TB_GETBUTTONW is not needed: only the style and id are read
$BTNS_WHOLEDROPDOWN = 0x0080
$BTNS_SEP = 0x0001

function Get-Child([IntPtr]$Hwnd, [string]$Class, [int]$Id = -1) {
    foreach ($row in [Ctl]::Children($Hwnd)) {
        $parts = $row -split '\|'
        if ($parts[0] -ne $Class) { continue }
        if ($Id -ge 0 -and [int]$parts[1] -ne $Id) { continue }
        return [IntPtr][int64]$parts[2]
    }
    return [IntPtr]::Zero
}

# One TBBUTTON, read a field at a time so the script does not have to lay the structure out.
function Read-ToolbarButtons([IntPtr]$Toolbar) {
    $count = [Ctl]::SendMessageW($Toolbar, $TB_BUTTONCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $size = 32   # sizeof(TBBUTTON) on x64: iBitmap, idCommand, fsState, fsStyle, bReserved, dwData, iString
    $buf = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($size)
    $rows = @()
    try {
        for ($i = 0; $i -lt $count; $i++) {
            [void][Ctl]::SendMessageW($Toolbar, $TB_GETBUTTON, [IntPtr]$i, $buf)
            $id = [System.Runtime.InteropServices.Marshal]::ReadInt32($buf, 4)
            $state = [System.Runtime.InteropServices.Marshal]::ReadByte($buf, 8)
            $style = [System.Runtime.InteropServices.Marshal]::ReadByte($buf, 9)
            $rows += [pscustomobject]@{ Index = $i; Id = $id; State = $state; Style = $style }
        }
    }
    finally { [System.Runtime.InteropServices.Marshal]::FreeHGlobal($buf) }
    return $rows
}

$results = @()
function Note([string]$What, [bool]$Ok, [string]$Saying) {
    $script:results += [pscustomobject]@{ What = $What; Ok = $Ok; Saying = $Saying }
    $mark = if ($Ok) { 'ok  ' } else { 'FAIL' }
    Write-Host ("{0} {1,-34} {2}" -f $mark, $What, $Saying)
}

$app = Start-App -Log $Log -Seconds $Seconds
$hwnd = $app.Hwnd
$appPid = $app.Pid
try {
    # --- The toolbar, without touching the mouse -----------------------------------------------
    $toolbar = Get-Child $hwnd 'ToolbarWindow32'
    $rebar = Get-Child $hwnd 'ReBarWindow32'
    Note 'toolbar is a real control' ($toolbar -ne [IntPtr]::Zero) "hwnd=$toolbar rebar=$rebar"
    if ($toolbar -ne [IntPtr]::Zero) {
        $buttons = Read-ToolbarButtons $toolbar
        $real = @($buttons | Where-Object { ($_.Style -band $BTNS_SEP) -eq 0 })
        Note 'eleven buttons and their groups' ($real.Count -eq 11) "$($real.Count) buttons, $($buttons.Count) items with separators"
        $drop = @($buttons | Where-Object { ($_.Style -band $BTNS_WHOLEDROPDOWN) -ne 0 })
        Note 'Open remote is a menu button' ($drop.Count -eq 1) "ids: $(($drop | ForEach-Object { $_.Id }) -join ',')"
    }

    # --- The chooser, opened from the menu it lives on ------------------------------------------
    $bar = Read-MenuBar $hwnd
    $item = Find-MenuItem $bar 'Format' 'Select columns'
    if ($null -eq $item) {
        Note 'Format has Select columns' $false 'not in the menu'
    }
    else {
        Note 'Format has Select columns' $true "id=$($item.Id)"
        Send-MenuCommand $hwnd $item
        $dlg = Wait-Dialog $appPid 'Select Columns' 6
        Note 'the chooser opens' ($dlg -ne [IntPtr]::Zero) "hwnd=$dlg"
        if ($dlg -ne [IntPtr]::Zero) {
            $list = Get-Child $dlg 'SysListView32'
            $rows = [Ctl]::SendMessageW($list, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()  # LVM_GETITEMCOUNT
            Note 'it lists the columns' ($rows -gt 0) "$rows rows"
            [void][Dlg]::PostMessageW($dlg, [Dlg]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 400
        }
    }

    # --- The two keys that were advertised and did nothing ---------------------------------------
    $wsh = New-Object -ComObject WScript.Shell
    [void](Set-Foreground $hwnd)
    $wsh.SendKeys('^k')
    $rules = Wait-Dialog $appPid 'Highlight rules' 6
    Note 'Ctrl+K opens the rules editor' ($rules -ne [IntPtr]::Zero) "hwnd=$rules"
    if ($rules -ne [IntPtr]::Zero) {
        [void][Dlg]::PostMessageW($rules, [Dlg]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 400
    }
    [void](Set-Foreground $hwnd)
    $wsh.SendKeys('{F1}')
    $keys = Wait-Dialog $appPid 'Keyboard' 6
    Note 'F1 opens the keyboard map' ($keys -ne [IntPtr]::Zero) "hwnd=$keys"
    if ($keys -ne [IntPtr]::Zero) {
        [void][Dlg]::PostMessageW($keys, [Dlg]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 400
    }
}
finally {
    Stop-App $app
}

Write-Host ''
$bad = @($results | Where-Object { -not $_.Ok })
Write-Host ("{0} checks, {1} failed" -f $results.Count, $bad.Count)
if ($bad.Count -gt 0) { exit 1 }
