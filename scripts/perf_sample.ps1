<#
.SYNOPSIS
    Samples one Diascia process and reports CPU time per second plus memory.

.DESCRIPTION
    Task Manager rounds a launcher's idle cost to 0%, so this measures the whole-window delta of
    TotalProcessorTime instead of per-sample differences: GetProcessTimes quantises to roughly
    15.6 ms, and 0.001% of a minute is well under one tick. Run it once per state (fully hidden,
    edge-collapsed, shown but idle) and compare the "cpu" line across builds.

.EXAMPLE
    powershell -File scripts/perf_sample.ps1 -Seconds 300 -Label collapsed
#>
param(
    [string]$Name = 'diascia',
    [int]$ProcessId = 0,
    [int]$Seconds = 300,
    [int]$IntervalMs = 250,
    [string]$Label = ''
)

$ErrorActionPreference = 'Stop'

$process = if ($ProcessId) {
    Get-Process -Id $ProcessId
} else {
    Get-Process -Name $Name | Select-Object -First 1
}
if (-not $process) {
    throw "no process named '$Name' is running"
}

$workingSet = [System.Collections.Generic.List[long]]::new()
$private = [System.Collections.Generic.List[long]]::new()

$process.Refresh()
$cpuStartMs = $process.TotalProcessorTime.TotalMilliseconds
$clock = [System.Diagnostics.Stopwatch]::StartNew()

while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
    Start-Sleep -Milliseconds $IntervalMs
    $process.Refresh()
    if ($process.HasExited) {
        throw "process $($process.Id) exited after $([int]$clock.Elapsed.TotalSeconds)s"
    }
    $workingSet.Add($process.WorkingSet64)
    $private.Add($process.PrivateMemorySize64)
}

$wallSeconds = $clock.Elapsed.TotalSeconds
$cpuMs = $process.TotalProcessorTime.TotalMilliseconds - $cpuStartMs

function Stat([System.Collections.Generic.List[long]]$values) {
    $sorted = $values | Sort-Object
    [pscustomobject]@{
        MinMB  = [math]::Round($sorted[0] / 1MB, 2)
        MeanMB = [math]::Round(($values | Measure-Object -Average).Average / 1MB, 2)
        MaxMB  = [math]::Round($sorted[-1] / 1MB, 2)
    }
}

$ws = Stat $workingSet
$pm = Stat $private

Write-Output ''
Write-Output "label        $Label"
Write-Output "pid          $($process.Id)"
Write-Output ("wall_s       {0:n1}" -f $wallSeconds)
Write-Output ("cpu          {0:n1} us/s  ({1:n4}% of one core)" -f ($cpuMs * 1000 / $wallSeconds), ($cpuMs / 10 / $wallSeconds))
Write-Output ("working_set  min {0} MB  mean {1} MB  max {2} MB" -f $ws.MinMB, $ws.MeanMB, $ws.MaxMB)
Write-Output ("private      min {0} MB  mean {1} MB  max {2} MB" -f $pm.MinMB, $pm.MeanMB, $pm.MaxMB)
Write-Output ("samples      {0}" -f $workingSet.Count)
