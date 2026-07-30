# Run a Phase-0 first-pixel experiment N times from cold and aggregate its CSV output.
#
# Each experiment writes one CSV line to %TEMP%\<OutFile> per process start, so the harness has to
# launch the process, wait for it to exit, and read the file between runs. Medians and ranges are
# reported per column because G3 found the variance (a 109 ms spread on an empty window) matters
# more than any single figure.
#
#   .\experiments\measure.ps1 -Exe target\release\g3-d2d.exe -OutFile g3-d2d-first-pixel.txt `
#       -Columns factory,window,target,draw,total
#
#   .\experiments\measure.ps1 -Exe target\release\g3-d3d11.exe -OutFile g3-d3d11-concurrent.txt `
#       -ExeArgs concurrent -Columns mode,window,device_wait,swapchain,draw,total,driver

param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$OutFile,
    [string]$ExeArgs = "",
    [int]$Runs = 7,
    [int]$GapMs = 250,
    [int]$TimeoutMs = 8000,
    [string]$Columns = ""
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Exe)) { throw "not found: $Exe" }
$exePath = (Resolve-Path $Exe).Path
$outPath = Join-Path $env:TEMP $OutFile
$procName = [System.IO.Path]::GetFileNameWithoutExtension($exePath)

Write-Host "exe   : $exePath"
Write-Host "size  : $((Get-Item $exePath).Length) bytes"
Write-Host "runs  : $Runs"
Write-Host ""

$rows = @()
for ($i = 1; $i -le $Runs; $i++) {
    if (Test-Path $outPath) { Remove-Item $outPath -Force }

    if ($ExeArgs -ne "") {
        $p = Start-Process -FilePath $exePath -ArgumentList $ExeArgs -PassThru
    } else {
        $p = Start-Process -FilePath $exePath -PassThru
    }

    # Poll for the report rather than waiting on exit: g3-d2d deliberately leaves its window open
    # (killing it is preferable to adding a PostQuitMessage, which would perturb the very binary
    # size G3 is measuring). g3-d3d11 self-terminates and will already be gone.
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while (-not (Test-Path $outPath) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 25
    }
    Start-Sleep -Milliseconds 100
    Get-Process -Id $p.Id -ErrorAction SilentlyContinue |
        Stop-Process -Force -Confirm:$false

    # CRITICAL: dispose the Process object from -PassThru. Without this, a subject that exits on its
    # own is never reaped -- PowerShell's cached handle keeps the process object alive as a zombie
    # that still holds its D3D device. Session 5 accumulated 33 of them and D3D11CreateDevice
    # degraded from 55 ms to 1210 ms as they piled up, silently corrupting every later measurement.
    try { $p.WaitForExit(2000) | Out-Null } catch {}
    try { $p.Dispose() } catch {}
    Start-Sleep -Milliseconds 100

    if (Test-Path $outPath) {
        $line = (Get-Content $outPath -Raw).Trim()
        if ($line) {
            Write-Host ("run {0,2}: {1}" -f $i, $line)
            $rows += , ($line -split ',')
        } else {
            Write-Warning "run $i produced an empty file"
        }
    } else {
        Write-Warning "run $i produced no output"
    }
    Start-Sleep -Milliseconds $GapMs
}

if ($rows.Count -eq 0) { throw "no runs produced output" }

Write-Host ""
$names = if ($Columns -ne "") { $Columns -split ',' } else { @() }
$width = ($rows | ForEach-Object { $_.Count } | Measure-Object -Maximum).Maximum

# p90 as well as the median, because SPEC 11.3 requires first-paint budgets to be stated as a
# percentile — G3 found a 109 ms spread on an empty window, so a mean or median alone is not a budget.
Write-Host ("{0,-14} {1,10} {2,10} {3,10} {4,10}" -f "column", "p50", "p90", "min", "max")
Write-Host ("-" * 60)
for ($c = 0; $c -lt $width; $c++) {
    $vals = @()
    foreach ($r in $rows) {
        if ($c -lt $r.Count) {
            $v = 0.0
            if ([double]::TryParse($r[$c], [ref]$v)) { $vals += $v }
        }
    }
    $label = if ($c -lt $names.Count) { $names[$c].Trim() } else { "col$c" }
    if ($vals.Count -eq 0) {
        # Non-numeric column (mode, driver) - report the distinct values instead.
        $distinct = ($rows | ForEach-Object { if ($c -lt $_.Count) { $_[$c] } } |
            Sort-Object -Unique) -join '/'
        Write-Host ("{0,-14} {1}" -f $label, $distinct)
        continue
    }
    $sorted = @($vals | Sort-Object)
    $n = $sorted.Count
    $p50 = $sorted[[int][math]::Floor(($n - 1) * 0.5)]
    $p90 = $sorted[[int][math]::Ceiling(($n - 1) * 0.9)]
    Write-Host ("{0,-14} {1,10:N3} {2,10:N3} {3,10:N3} {4,10:N3}" -f
        $label, $p50, $p90, $sorted[0], $sorted[-1])
}
