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

.EXAMPLE
    # 300 rows, each carrying a ~24 KiB icon, so private bytes are comparable across builds.
    powershell -File scripts/probe.ps1 -Seed -Perf -Seconds 120 -Entries 300 -BlobsKb 24
#>
param(
    [int]$Seconds = 20,
    [switch]$Perf,
    [switch]$Seed,
    [string]$DataDir = 'target/probe-data',
    [string]$Csv = 'target/probe.csv',
    [string]$Label = '',
    [int]$Entries = 0,
    [int]$BlobsKb = 0
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
    # Memory readings are only comparable when both runs carry the same dataset, so the row count
    # and the icon blob size are pinned here rather than left to whatever shell extraction returns.
    if ($Entries) { $env:DIASCIA_PROBE_ENTRIES = "$Entries" }
    if ($BlobsKb) { $env:DIASCIA_PROBE_BLOBS_KB = "$BlobsKb" }
    cargo test --release --test probe_seed -- --ignored --nocapture | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'probe_seed failed' }
    Remove-Item Env:DIASCIA_PROBE_SEED
    'DIASCIA_PROBE_ENTRIES', 'DIASCIA_PROBE_BLOBS_KB' | ForEach-Object {
        if (Test-Path "Env:$_") { Remove-Item "Env:$_" }
    }
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
    'icon_cache_bytes', 'entry_blob_bytes', 'working_set_kb', 'private_kb', 'full_frame',
    'region_set', 'refresh_ms', 'icon_index_ms', 'entries_query_ms', 'save_entry_ms',
    'store_export_ms', 'store_validate_ms', 'startup_prepare_ms', 'startup_store_ms',
    'startup_backend_ms', 'startup_window_ms', 'startup_signals_ms', 'startup_hotkey_ms',
    'startup_app_ms', 'startup_callbacks_ms', 'startup_ready_ms'
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
$responses = @()
$slow = 0
foreach ($summon in ($rows | Where-Object { $_.event -eq 'hotkey_received' })) {
    # The application-side half is always measurable: show_requested is recorded inside the same
    # turn the hotkey arrives.
    $shown = $rows | Where-Object { $_.event -eq 'show_requested' -and $_.at -ge $summon.at } | Select-Object -First 1
    if ($shown) { $responses += [math]::Round($shown.at - $summon.at, 1) }
    $frame = $rows | Where-Object { $_.event -eq 'frame_ms' -and $_.at -ge $summon.at } | Select-Object -First 1
    if ($frame -and ($frame.at - $summon.at) -lt 1000) {
        $reveals += [math]::Round($frame.at - $summon.at, 1)
    }
    else {
        # Summoning an already-mapped window can legitimately repaint nothing, and the next frame
        # row then belongs to a much later scroll. Reporting that gap as latency is a lie, so it is
        # counted separately instead.
        $slow++
    }
}
if ($responses.Count) {
    $sorted = @($responses | Sort-Object)
    Write-Output ("summon hotkey->show_requested: n={0}  p50={1}ms  max={2}ms" -f $sorted.Count,
        $sorted[[int][math]::Floor(($sorted.Count - 1) * 0.5)], $sorted[-1])
}
if ($reveals.Count) {
    $sorted = @($reveals | Sort-Object)
    Write-Output ("summon hotkey->first frame: n={0}  p50={1}ms  max={2}ms" -f $sorted.Count,
        $sorted[[int][math]::Floor(($sorted.Count - 1) * 0.5)], $sorted[-1])
}
if ($slow) {
    Write-Output ("summon: $slow of them had no frame within 1s, i.e. nothing was rasterised to " +
        'prove the paint; scroll or resize once after each summon to sample it.')
}
if (-not $reveals.Count -and -not $responses.Count) {
    Write-Output 'reveal: no hotkey_received row, so the launcher was never summoned by keyboard'
}

# Events where the value is a constant and the row count is the actual measurement.
foreach ($name in 'full_frame', 'region_set', 'refresh_ms') {
    $count = @($rows | Where-Object { $_.event -eq $name }).Count
    Write-Output ("{0,-14} n={1}" -f "$name ", $count)
}

# Which call rebuilt a window frame, and when. corner_watch.ps1 reports the same clock in its
# app_elapsed_ms column, so a visible corner or shadow change can be matched against a specific
# write instead of guessed at. No rows here is the expected result for a docked session.
$writes = @($rows | Where-Object { $_.event -eq 'frame_write' })
$writeNames = @{
    1 = 'hand shape back';   2 = 'ex-style';         3 = 'winit buttons';
    4 = 'immersive dark';    5 = 'corner/nc block';  6 = 'winit skip-taskbar';
    7 = 'custom frame style';8 = 'border colour';    9 = 'square for clip';
    10 = 'clip region';      11 = 'z-band';          12 = 'layered opacity'
}
if ($writes.Count) {
    Write-Output "frame_write  n=$($writes.Count)  (each of these makes DWM redraw the non-client area)"
    $writes | Group-Object { [int]$_.value } | Sort-Object { [int]$_.Name } | ForEach-Object {
        $at = @($_.Group | ForEach-Object { [int]$_.at })
        $shown = @($at | Select-Object -First 12) -join ' '
        if ($at.Count -gt 12) {
            $shown += " ... (+$($at.Count - 12) more)"
        }
        $label = if ($writeNames.ContainsKey([int]$_.Name)) { $writeNames[[int]$_.Name] } else { "code $($_.Name)" }
        Write-Output ("  {0,-20} n={1,-4} at(ms): {2}" -f $label, $_.Count, $shown)
    }
}

# Startup timeline. `startup_*` rows are offsets from the start of run_at, so the phase cost is
# the delta against the previous mark. Durations (store_export_ms, refresh_ms, ...) already appear
# in the table above. first_frame is the elapsed_ms of the first rasterised frame, on the
# metrics::init() origin, i.e. how long process start to visible pixels really took.
$phases = @($rows | Where-Object { $_.event -like 'startup_*' } | Sort-Object value)
if ($phases.Count) {
    Write-Output 'startup timeline (ms since run_at, and cost of that phase):'
    $previous = 0.
    foreach ($phase in $phases) {
        Write-Output ("  {0,-20} {1,8:N1}   +{2,7:N1}" -f $phase.event, $phase.value, ($phase.value - $previous))
        $previous = $phase.value
    }
}
$first = $rows | Where-Object { $_.event -eq 'frame_ms' } | Sort-Object at | Select-Object -First 1
if ($first) {
    Write-Output ("  {0,-20} {1,8:N1}" -f 'first_frame', $first.at)
}
