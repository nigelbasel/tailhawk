# Proves the menu's dialogs are real windows, not sheets drawn on the grid.
#
# The owner's report, 2026-08-24: About "just dumps the text on top of the window", and every Help
# item behaved like a dropdown. Their report of 2026-08-25 was the same objection about Format's
# define-from-a-line sheet - "yet another terrible UI". The one thing an overlay can never do is put
# a second top-level window of the dialog class `#32770` on the desktop, so that is what this checks:
# choose the menu item, wait for a real dialog with the right caption owned by *this* process, close
# it, and confirm the application survived.
#
#   powershell tools/verify-dialogs.ps1
#   powershell tools/verify-dialogs.ps1 -Only Format
#
# **What changed on 2026-08-26.** The old version drove a menu bar the renderer drew, taking its
# geometry from `TAILHAWK_DUMP_MENU_HITS` and naming its targets by position - "menu 6, the last
# row". The bar is native now, that debug channel is deleted, and this script had been failing with
# *"menu drew no entries"* against an application whose menus were fine. It now reads the real
# `HMENU` through `Menu.ps1` and names every target the way a person would: `Help` then `About`.
# Positions were never the right handle anyway - the recent-files block silently shifts every row
# of the File menu the moment a file has been opened.
#
# **It no longer touches the keyboard, so it can run while the desktop is in use.** Choosing a menu
# item *is* a `WM_COMMAND` carrying that item's id, and the id is read off the live menu rather than
# from a table here - so the message is the one the menu would have sent. Windows' own popup
# tracking is the only thing skipped, and that is not Tailhawk's code. Driving the menu with the
# keys is `verify-menus.ps1`'s job, because there the menu path is the thing under test.
[CmdletBinding()]
param(
    [string]$Only = '',
    [string]$Log = "$env:TEMP\tailhawk-verify-dialogs.log"
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')
. (Join-Path $PSScriptRoot 'Menu.ps1')
. (Join-Path $PSScriptRoot 'Dialog.ps1')

# Heading, item, and the caption the dialog must carry. The captions are the ones the templates
# name (`dialog.rs`), and the two that are not ours - Font from `ChooseFontW`, About from
# `TaskDialogIndirect` - are what Windows titles them.
$CASES = @(
    @{ Menu = 'Help';     Item = 'About';           Title = 'About Tailhawk' }
    @{ Menu = 'Help';     Item = 'Keyboard map';    Title = 'Keyboard map' }
    @{ Menu = 'Settings'; Item = 'Preferences';     Title = 'Preferences' }
    @{ Menu = 'Format';   Item = 'Font';            Title = 'Font' }
    @{ Menu = 'Format';   Item = 'Define from a';   Title = 'Define Format' }
    @{ Menu = 'Format';   Item = 'Import layout';   Title = 'Import Layout' }
    @{ Menu = 'Edit';     Item = 'Filter: include'; Title = 'Add Filter' }
    @{ Menu = 'Edit';     Item = 'Find';            Title = 'Find' }
    @{ Menu = 'View';     Item = 'Go to line';      Title = 'Go to line' }
)

if (-not (Test-Path $Log)) {
    $sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
    for ($i = 0; $i -lt 200; $i++) {
        $sw.WriteLine("2026-08-24 09:14:02.117 INFO  Api.Controller line $i returned 412 rows in 88ms")
    }
    $sw.Close()
}

Get-Process tailhawk -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

$wsh = New-Object -ComObject WScript.Shell
$proc = Start-Tailhawk $Log
$fails = @()

try {
    $bar = Read-MenuBar $proc.MainWindowHandle

    foreach ($case in $CASES) {
        $name = "$($case.Menu) > $($case.Item)"
        if ($Only -and $case.Menu -notlike "$Only*") { continue }

        $item = Find-MenuItem $bar $case.Menu $case.Item
        if (-not $item.Enabled) {
            Write-Host "$name : FAIL -- the item is greyed, so the dialog cannot be reached"
            $fails += $name
            continue
        }

        Send-MenuCommand $proc.MainWindowHandle $item
        try {
            $dlg = Wait-Dialog $proc.Id $case.Title
        } catch {
            Write-Host "$name : FAIL -- no '#32770' titled '$($case.Title)' appeared"
            Write-Host "  this process's windows: $([Dlg]::Windows($proc.Id) -join ' / ')"
            $fails += $name
            continue
        }
        $rect = Get-DialogRect $dlg
        Write-Host "$name : native dialog '$($case.Title)' at $($rect.W)x$($rect.H)"

        if (-not (Close-Dialog $proc.Id $case.Title)) {
            Write-Host "$name : FAIL -- the dialog did not close"
            $fails += $name
            continue
        }
        $proc.Refresh()
        if ($proc.HasExited) {
            Write-Host "$name : FAIL -- closing the dialog took the application with it"
            $fails += $name
            break
        }
        Start-Sleep -Milliseconds 300
    }
} finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}

if ($fails.Count -eq 0) {
    Write-Host 'PASS: every menu dialog is a real native window, and closes without taking the app'
} else {
    Write-Host "FAIL: $($fails -join ', ')"
    exit 1
}
