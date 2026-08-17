# Opens the shipped binary on a log that exercises the semantic catalogue and reads the colours
# off the screen -- E23's version of the check `verify-find.ps1` makes for search.
#
# The unit tests prove the catalogue produces the right spans; this proves the spans reach pixels
# in the shipped binary, in the colours `semantic.rs` names, and that a screenful of them costs a
# frame what §11.3 allows. It counts pixels of each colour rather than comparing images, so a font
# or DPI change does not fail it.
#
#   powershell tools/verify-semantic.ps1
[CmdletBinding()]
param(
    [int]$Lines = 20000,
    [string]$Log = "$env:TEMP\tailhawk-verify-semantic.log",
    [string]$Shot = "$env:TEMP\tailhawk-verify-semantic.png",
    [switch]$KeepWindow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Screen.ps1')

# One of each thing the catalogue colours, on every few lines, so any screenful has all of them.
Write-Host "writing $Lines lines to $Log"
$sw = [System.IO.StreamWriter]::new($Log, $false, [System.Text.UTF8Encoding]::new($false))
for ($i = 0; $i -lt $Lines; $i++) {
    $stamp = '2026-08-16 09:14:{0:D2}.{1:D3}' -f ($i % 60), ($i % 1000)
    switch ($i % 5) {
        0 { $sw.WriteLine("$stamp ERROR Api.Dispatch from 10.0.0.$($i % 250) job=$i failed after 30000ms status=503") }
        1 { $sw.WriteLine("$stamp WARN  Api.Sql slow query took 1240ms for /api/users/$i") }
        2 { $sw.WriteLine("$stamp INFO  Api.Controller returned 412 rows in 88ms id=3f2504e0-4f89-11d3-9a0c-0305e82c3301") }
        3 { $sw.WriteLine("$stamp DEBUG Api.Http GET https://api.example.com/v1/items?page=$i `"cache miss`"") }
        4 { $sw.WriteLine("$stamp INFO  Api.Controller line $i returned 412 rows in 88ms") }
    }
}
$sw.Close()

$proc = Start-Tailhawk $Log
try {
    $hwnd = $proc.MainWindowHandle
    $wsh = New-Object -ComObject WScript.Shell

    # Sixty pages of scrolling, so the frame instrument's p95 is a percentile of frames that laid
    # out and coloured a fresh screenful, and not -- as it is over a handful of frames -- the cold
    # first one. Then a line appended to the log: the title, and the instrument in it, is only
    # re-rendered when something changes, and a follow tick that sees growth is the change a tail
    # is for.
    1..60 | ForEach-Object { $wsh.SendKeys('{PGDN}'); Start-Sleep -Milliseconds 40 }
    # Appended the way a logger appends -- a shared-write handle -- because `Add-Content` asks for
    # exclusive access to the file the window is tailing, which is the one thing a tail forbids.
    $fs = [System.IO.File]::Open($Log, [System.IO.FileMode]::Append, [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes("2026-08-16 09:15:00.000 INFO  Api.Controller appended after the scroll`n")
    $fs.Write($bytes, 0, $bytes.Length); $fs.Close()
    $null = Wait-For { $proc.Refresh(); $proc.MainWindowTitle -match "$($Lines + 1) lines" } 'the follow tick to notice the appended line'
    Start-Sleep -Milliseconds 200
    $proc.Refresh()
    $title = $proc.MainWindowTitle
    Write-Host "title:   $title"

    $bmp = [Shot]::Client($hwnd)
    $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
    # The colours `semantic.rs` names, as the render target writes them.
    $counts = [ordered]@{}
    foreach ($entry in @(
        @('timestamp', 0.52, 0.64, 0.78),
        @('error',     0.96, 0.47, 0.38),
        @('warn',      0.93, 0.73, 0.32),
        @('debug',     0.56, 0.59, 0.64),
        @('number',    0.61, 0.79, 0.94),
        @('duration',  0.60, 0.83, 0.63),
        @('ip',        0.46, 0.82, 0.80),
        @('url',       0.47, 0.70, 0.96),
        @('path',      0.76, 0.71, 0.94),
        @('quoted',    0.86, 0.77, 0.58),
        @('key',       0.63, 0.71, 0.79),
        @('ink',       0.878, 0.890, 0.906)
    )) {
        $rgb = ConvertFrom-Rgbf $entry[1] $entry[2] $entry[3]
        $counts[$entry[0]] = [Shot]::Count($bmp, $rgb[0], $rgb[1], $rgb[2], 8)
    }
    $bmp.Dispose()

    Write-Host ''
    Write-Host "screenshot: $Shot"
    $counts.GetEnumerator() | ForEach-Object { Write-Host ("{0,-10} {1,7} px" -f $_.Key, $_.Value) }

    $failures = @()
    foreach ($name in 'timestamp', 'error', 'warn', 'number', 'duration', 'ip', 'url', 'ink') {
        if ($counts[$name] -lt 50) { $failures += "no $name colour on screen ($($counts[$name]) px)" }
    }
    if ($title -match 'frame p95 ([\d.]+) ms') {
        $p95 = [double]$Matches[1]
        Write-Host ("frame p95:  {0} ms" -f $p95)
        if ($p95 -gt 16.7) { $failures += "frame p95 $p95 ms is over the 16.67 ms budget with the catalogue on" }
    } else {
        $failures += 'the title carries no frame instrument to read'
    }
    if ($proc.HasExited) { $failures += 'the process exited' }
    if ($failures) {
        $failures | ForEach-Object { Write-Host "FAIL: $_" -ForegroundColor Red }
        $failed = $true
    } else {
        Write-Host 'PASS' -ForegroundColor Green
    }
}
finally {
    # An `exit` inside the try would skip this block and leak the window; the exit code waits.
    if (-not $KeepWindow -and -not $proc.HasExited) { $proc.Kill() }
}
if ($failed) { exit 1 }
