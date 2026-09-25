<#
.SYNOPSIS
    Runs a throwaway launcher instance and summarises its metrics. Sends no input at all.

.DESCRIPTION
    Why there is no -Keys switch: the earlier way of measuring interaction latency synthesised
    mouse and keyboard events against the live launcher, which moved the user's pointer and clicked
    whatever happened to be under it. Everything that can be measured without input is measured
    here; anything that needs a summon, a scroll or a keystroke is done by a human while this
    script samples, and read back out of the CSV afterwards.

    SAFETY: -DataDir must be a throwaway directory. A second instance aimed at the *real* data dir
    signals the running launcher to come to the foreground (platform::Instance::acquire) and then
    exits, i.e. it would hijack the user's own launcher.

    Metrics are flushed on the 500ms interaction timer, so stopping the process loses at most that
    many rows. The seeded SQLite file is left to WAL recovery, which is fine for a throwaway dir.

    Kept ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII literals arrive mangled.

.EXAMPLE
    powershell -File scripts/probe.ps1 -Seconds 30

.EXAMPLE
    powershell -File scripts/probe.ps1 -Perf -Seconds 300 -Label collapsed
#>
param(
    [int]$Seconds = 20,
    [switch]$Perf,
    [switch]$Seed,
    [string]$DataDir = 'target/probe-data',
    [string]$Csv = 'target/probe.csv',
    [string]$Label = ''
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$exe = 'target/release/diascia.exe'
if (-not (Test-Path $exe)) {
    throw 'build it first: cargo build --release --locked'
}

if ($Seed -or -not (Test-Path (Join-Path $DataDir 'diascia.db'))) {
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    $env:DIASCIA_PROBE_SEED = Join-Path (Get-Location) $DataDir
    cargo test --release --test probe_seed -- --ignored --nocapture | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'probe_seed failed' }
    Remove-Item Env:DIASCIA_PROBE_SEED
}

if (Test-Path $Csv) {
    # metrics.rs opens the CSV in append mode, so a stale file would blend two runs.
    Remove-Item $Csv
}
$env:DIASCIA_METRICS = $Csv
$process = Start-Process $exe -ArgumentList @('--data-dir', $DataDir) -PassThru
Write-Output "pid $($process.Id)   data dir $DataDir"
Write-Output 'Operate the launcher yourself now (summon, scroll, type, collapse). This script sends no input.'

if ($Perf) {
    Read-Host 'Put the window in the state to sample (hidden / edge-collapsed / shown idle), then Enter'
    $tag = if ($Label) { $Label } else { 'probe' }
    & (Join-Path $PSScriptRoot 'perf_sample.ps1') -ProcessId $process.Id -Seconds $Seconds -Label $tag
}
else {
    Start-Sleep -Seconds $Seconds
}

Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 600
Remove-Item Env:DIASCIA_METRICS

if (-not (Test-Path $Csv)) {
    throw "no metrics in $Csv - startup may have failed (the CSV is only written on the interaction timer)"
}
$events = Import-Csv -Path $Csv -Header elapsed_ms, event, value
$expected = @(
    'frame_ms', 'animation_frame_ms', 'scroll_step_us', 'icon_request_us', 'evaluate_us',
    'search_render_us', 'interaction_tick', 'show_requested', 'hotkey_received', 'entry_count',
    'icon_cache_bytes', 'entry_blob_bytes'
)
$events | Group-Object event | ForEach-Object {
    $values = @($_.Group | ForEach-Object { [double]$_.value } | Sort-Object)
    [pscustomobject]@{
        event = $_.Name
        n     = $values.Count
        p50   = [math]::Round($values[[int][math]::Floor(($values.Count - 1) * 0.5)], 3)
        p95   = [math]::Round($values[[int][math]::Floor(($values.Count - 1) * 0.95)], 3)
        max   = [math]::Round($values[-1], 3)
    }
} | Sort-Object event | Format-Table -AutoSize

$seen = @($events | ForEach-Object { $_.event })
$missing = @($expected | Where-Object { $seen -notcontains $_ })
if ($missing.Count) {
    Write-Output ("no rows for: {0}" -f ($missing -join ', '))
}

# Reveal latency, measured the only honest way: the CSV gap between the hotkey arriving and the
# first frame rasterised after it. A script wall-clock measurement would include the DWM foreground
# switch and read 60-85ms for the same frame.
$rows = @($events | ForEach-Object {
    [pscustomobject]@{ at = [double]$_.elapsed_ms; event = $_.event; value = [double]$_.value }
})
$reveals = @()
foreach ($summon in ($rows | Where-Object { $_.event -eq 'hotkey_received' })) {
    $frame = $rows | Where-Object { $_.event -eq 'frame_ms' -and $_.at -ge $summon.at } | Select-Object -First 1
    if ($frame) {
        $reveals += [math]::Round($frame.at - $summon.at, 1)
    }
}
if ($reveals.Count) {
    $sorted = @($reveals | Sort-Object)
    Write-Output ("reveal hotkey->first frame: n={0}  p50={1}ms  max={2}ms" -f $sorted.Count,
        $sorted[[int][math]::Floor(($sorted.Count - 1) * 0.5)], $sorted[-1])
}
else {
    Write-Output 'reveal: no hotkey_received row, so the launcher was never summoned by keyboard'
}
