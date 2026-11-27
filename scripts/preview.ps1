<#
.SYNOPSIS
    Headless UI preview generator wrapper for Raptim.

.DESCRIPTION
    Renders Raptim window, overlay and settings previews completely offscreen using
    Slint's software renderer without creating OS windows or stealing foreground focus.
    Kept ASCII-only for Windows PowerShell 5.1 compatibility.

.EXAMPLE
    powershell -File scripts/preview.ps1

.EXAMPLE
    powershell -File scripts/preview.ps1 -Preset all -Theme both -Frame card
#>
param(
    [string]$OutDir = 'target/ui-previews',
    [ValidateSet('zh', 'en')][string]$Lang = 'zh',
    [ValidateSet('both', 'light', 'dark')][string]$Theme = 'both',
    [ValidateSet('showcase', 'all')][string]$Preset = 'showcase',
    [string]$Scenes = '',
    [double]$Scale = 2.0,
    [ValidateSet('window', 'card', 'raw')][string]$Frame = 'window',
    [string]$DataDir = '',
    [string]$ImportJson = '',
    [switch]$DebugBuild,
    [switch]$NoGallery,
    [switch]$NoOverview,
    [switch]$List,
    [switch]$Open,
    [string]$TargetDir = '',
    [int]$Jobs = 0
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$prevTargetDir = $env:CARGO_TARGET_DIR
if ($TargetDir) {
    $env:CARGO_TARGET_DIR = $TargetDir
}

$cargoArgs = @('run')
if (-not $DebugBuild) {
    $cargoArgs += '--release'
}
$cargoArgs += @('--locked', '--example', 'ui_preview')
if ($Jobs -gt 0) {
    $cargoArgs += @('-j', "$Jobs")
}
$cargoArgs += '--'

if ($List) {
    $cargoArgs += @('--list', '--lang', $Lang)
} else {
    $cargoArgs += @('--out', $OutDir, '--lang', $Lang, '--theme', $Theme, '--preset', $Preset, '--scale', "$Scale", '--frame', $Frame)
    if ($Scenes) {
        $cargoArgs += @('--scenes', $Scenes)
    }
    if ($DataDir) {
        $cargoArgs += @('--data-dir', $DataDir)
    }
    if ($ImportJson) {
        $cargoArgs += @('--import-json', $ImportJson)
    }
    if ($NoGallery) {
        $cargoArgs += '--no-gallery'
    }
    if ($NoOverview) {
        $cargoArgs += '--no-overview'
    }
}

try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "ui_preview failed (exit $LASTEXITCODE)"
    }
    if ($Open -and -not $List -and -not $NoGallery) {
        $gallery = Join-Path $OutDir 'index.html'
        if (Test-Path $gallery) {
            Start-Process $gallery
        }
    }
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
