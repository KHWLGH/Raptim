<#
.SYNOPSIS
    The headless gate: formatting, lints, unit tests and a release build.

.DESCRIPTION
    Raptim's default verification path. Runs rustfmt, clippy, unit tests, and a release build.
    Kept ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII literals arrive mangled.

.EXAMPLE
    powershell -File scripts/verify.ps1

.EXAMPLE
    powershell -File scripts/verify.ps1 -Release
#>
param(
    [switch]$Release,
    [string]$TargetDir = '',
    [int]$Jobs = 0
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$prevTargetDir = $env:CARGO_TARGET_DIR
if ($TargetDir) {
    $env:CARGO_TARGET_DIR = $TargetDir
}
$resolvedTargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }

function Invoke-Gate([string]$name, [scriptblock]$body) {
    Write-Output "==> $name"
    $prevPref = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $captured = & $body 2>&1 | ForEach-Object {
            $line = if ($_ -is [System.Management.Automation.ErrorRecord]) {
                $_.Exception.Message
            } else {
                "$_"
            }
            Write-Output $line
            $line
        }
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $prevPref
    }
    if ($code -ne 0) {
        $joined = $captured -join "`n"
        if ($joined -match 'os error 32|Access is denied|build-script-build') {
            $resolved = Resolve-Path $resolvedTargetDir -ErrorAction SilentlyContinue
            if (-not $resolved) { $resolved = $resolvedTargetDir }
            Write-Warning ("Build file lock detected (likely real-time AV scanning '{0}'). Add '{1}', '~/.cargo', and '~/.rustup' to AV exclusions, remove any quarantined build-script-build.exe, or re-run with -TargetDir <excluded_dir> -Jobs 1." -f $resolvedTargetDir, $resolved)
        }
        throw "$name failed (exit $code)"
    }
}

# += and not an if-expression: PowerShell unwraps a single-element array that comes out of
# the pipeline, and splatting the resulting string hands cargo one argument per character.
$cargoProfile = @()
if ($Release) {
    $cargoProfile += '--release'
}
$cargoJobs = @()
if ($Jobs -gt 0) {
    $cargoJobs += @('-j', "$Jobs")
}

try {
    Invoke-Gate 'rustfmt' { cargo fmt --check }
    Invoke-Gate 'clippy' { cargo clippy @cargoProfile @cargoJobs --all-targets --locked -- -D warnings }
    Invoke-Gate 'test' { cargo test @cargoProfile @cargoJobs --all-targets --locked }
    Invoke-Gate 'release build' { cargo build --release @cargoJobs --locked }

    $exe = Get-Item (Join-Path $resolvedTargetDir 'release/raptim.exe')
    Write-Output ''
    Write-Output ("exe    {0:n0} bytes   built {1}" -f $exe.Length, $exe.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))
    Write-Output 'gate passed: no step above brought a window to the foreground.'
}
finally {
    if ($TargetDir) {
        if ($null -eq $prevTargetDir) {
            Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_TARGET_DIR = $prevTargetDir
        }
    }
}
