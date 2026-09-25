<#
.SYNOPSIS
    The headless gate: formatting, lints, tests and a release build. Nothing here touches the desktop.

.DESCRIPTION
    Diascia's default verification path. Every check runs offscreen: the UI tests install a
    MinimalSoftwareWindow platform and rasterise into their own buffer, the menu tests read the tree
    as plain data, and the owner-draw probes paint into a memory bitmap. So this can run while the
    user works in another window.

    -Machine opts into the real-window regression, which is a different thing: it shows the launcher
    and editor windows, focuses them, puts a tray icon down and registers Ctrl+Alt+Shift+F10/F9.
    Run it deliberately, never inside a loop.

    `cargo fmt` never inspects src/ui_commands.rs or src/app_session.rs: they are include!ed into
    main.rs, so they are not part of rustfmt's module walk. The second check covers them.

    Kept ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII literals arrive mangled.

.EXAMPLE
    powershell -File scripts/verify.ps1

.EXAMPLE
    powershell -File scripts/verify.ps1 -Release -Machine
#>
param(
    [switch]$Release,
    [switch]$Machine
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Invoke-Gate([string]$name, [scriptblock]$body) {
    Write-Output "==> $name"
    & $body
    if ($LASTEXITCODE -ne 0) {
        throw "$name failed (exit $LASTEXITCODE)"
    }
}

# += and not an if-expression: PowerShell unwraps a single-element array that comes out of
# the pipeline, and splatting the resulting string hands cargo one argument per character.
$cargoProfile = @()
if ($Release) {
    $cargoProfile += '--release'
}

Invoke-Gate 'rustfmt (crate modules)' { cargo fmt --check }
Invoke-Gate 'rustfmt (include!ed files)' {
    rustfmt --edition 2024 --check src/ui_commands.rs src/app_session.rs
}
Invoke-Gate 'clippy' { cargo clippy @cargoProfile --all-targets --locked -- -D warnings }
Invoke-Gate 'test (all offscreen)' { cargo test @cargoProfile --all-targets --locked }
if ($Machine) {
    Invoke-Gate 'machine pass (this one takes the foreground)' {
        cargo test --release --locked -- --ignored --nocapture
    }
}
Invoke-Gate 'release build' { cargo build --release --locked }

$exe = Get-Item target/release/diascia.exe
Write-Output ''
Write-Output ("exe    {0:n0} bytes   built {1}" -f $exe.Length, $exe.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))
Write-Output 'gate passed: no step above brought a window to the foreground.'
