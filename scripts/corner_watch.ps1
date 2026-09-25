<#
.SYNOPSIS
    Watches the four corners of the launcher's native window while a human slides it. Sends no input.

.DESCRIPTION
    Why this exists: DWM rounding cannot be proven offscreen (see the verification layers in README),
    and the metrics CSV already showed the work-area HRGN path never runs on a single display - so
    "the corners pop around a dock slide" had no attribution. This script supplies it.

    It samples the screen, never the window: PrintWindow and GetDC(hwnd) both hand back the client
    surface, which has no corners, because DWM applies the mask in the compositor. So the corner of
    the GetWindowRect box is read straight off the desktop.

    Per tick it grabs a Patch x Patch block at each corner and walks the diagonal inwards from the
    extreme pixel, reporting how many pixels are not window content before the content starts. The
    reference colour is taken from the inner end of the same diagonal, so neither the theme nor the
    wallpaper has to be known. Rounded gives a run of about 0.29 x radius (Windows' 8 px logical
    radius is 16 physical at 200% DPI, so roughly 5); squared gives 0, because content reaches the
    very corner pixel.

    WHAT TO DO: start it, then do one collapse, one reveal, and one hotkey summon, keeping the window
    on screen. Read the flip lines: a flip on a tick with moving=0 points at the calls that run when
    the slide settles, a flip while moving=1 points at the per-frame window moves, and a flip only
    while the reveal strip lies on the screen edge is the operating system's own frame handling.

    SAFETY: read-only. No SetCursorPos, no SendInput, no window-moving API - the same rule probe.ps1
    follows, because an earlier latency harness moved the user's pointer and clicked whatever happened
    to be under it.

    Kept ASCII-only: Windows PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII literals arrive mangled.

.EXAMPLE
    powershell -File scripts/corner_watch.ps1

.EXAMPLE
    powershell -File scripts/corner_watch.ps1 -Seconds 20 -IntervalMs 3
#>
param(
    [string]$Title = 'Diascia',
    [string]$Name = 'diascia',
    [int]$ProcessId = 0,
    [int]$Seconds = 12,
    [int]$IntervalMs = 5,
    [int]$Patch = 28,
    [int]$Tolerance = 14,
    [string]$Csv = 'target/corner-watch.csv',
    [string]$Out = 'target/corner-watch'
)
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing
[void] (Add-Type -PassThru -TypeDefinition @'
namespace DiasciaProbe {
    using System;
    using System.Runtime.InteropServices;
    using System.Text;
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public static class Native {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int maxCount);
    }
}
'@)

# Without this every coordinate below is virtualised by the shell and the samples come from the
# wrong part of the screen.
if (-not [DiasciaProbe.Native]::SetProcessDpiAwarenessContext([IntPtr](-4))) {
    [void][DiasciaProbe.Native]::SetProcessDPIAware()
}

$sample = New-Object DiasciaProbe.RECT
# SM_XVIRTUALSCREEN/SM_YVIRTUALSCREEN/SM_CXVIRTUALSCREEN/SM_CYVIRTUALSCREEN: the union of every
# display, negative origins included, which is what "can this corner be read off the desktop" means.
$virtual = New-Object System.Drawing.Rectangle(
    [DiasciaProbe.Native]::GetSystemMetrics(76),
    [DiasciaProbe.Native]::GetSystemMetrics(77),
    [DiasciaProbe.Native]::GetSystemMetrics(78),
    [DiasciaProbe.Native]::GetSystemMetrics(79))

function Find-LauncherWindow([int]$targetPid) {
    # Settings and editor windows belong to the same process, and a collapsed launcher shares the
    # desktop with a 26x26 taskbar-restore window of its own. Title plus area picks the one window
    # this script may read.
    $found = [System.Collections.Generic.List[System.IntPtr]]::new()
    $callback = {
        param([System.IntPtr]$hWnd, [System.IntPtr]$lParam)
        $owner = [uint32]0
        [void][DiasciaProbe.Native]::GetWindowThreadProcessId($hWnd, [ref]$owner)
        if ($owner -eq [uint32]$targetPid -and [DiasciaProbe.Native]::IsWindowVisible($hWnd)) {
            $text = New-Object System.Text.StringBuilder 64
            [void][DiasciaProbe.Native]::GetWindowTextW($hWnd, $text, $text.Capacity)
            $rect = New-Object DiasciaProbe.RECT
            if ($text.ToString() -eq $Title -and [DiasciaProbe.Native]::GetWindowRect($hWnd, [ref]$rect)) {
                if (($rect.Right - $rect.Left) * ($rect.Bottom - $rect.Top) -gt 10000) {
                    $found.Add($hWnd) | Out-Null
                }
            }
        }
        return $true
    }
    [void][DiasciaProbe.Native]::EnumWindows($callback, [IntPtr]::Zero)
    return $found
}

function Read-Pixel($data, [int]$x, [int]$y) {
    [System.Runtime.InteropServices.Marshal]::ReadInt32($data.Scan0.ToInt64() + ($y * $data.Stride) + ($x * 4))
}

function Measure-CornerRun([System.Drawing.Bitmap]$bitmap, [bool]$fromLeft, [bool]$fromTop, [int]$tolerance) {
    $bounds = New-Object System.Drawing.Rectangle(0, 0, $bitmap.Width, $bitmap.Height)
    $data = $bitmap.LockBits($bounds, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $size = $bitmap.Width
        # The inner end of this corner's own diagonal, i.e. deep inside the window.
        $reference = Read-Pixel $data $(if ($fromLeft) { $size - 3 } else { 2 }) $(if ($fromTop) { $size - 3 } else { 2 })
        $r0 = $reference -band 0xff
        $g0 = ($reference -shr 8) -band 0xff
        $b0 = ($reference -shr 16) -band 0xff
        for ($i = 0; $i -lt $size; $i++) {
            $x = if ($fromLeft) { $i } else { $size - 1 - $i }
            $y = if ($fromTop) { $i } else { $size - 1 - $i }
            $p = Read-Pixel $data $x $y
            if ([math]::Abs(($p -band 0xff) - $r0) -le $tolerance -and
                [math]::Abs((($p -shr 8) -band 0xff) - $g0) -le $tolerance -and
                [math]::Abs((($p -shr 16) -band 0xff) - $b0) -le $tolerance) {
                return $i
            }
        }
        return $size
    }
    finally {
        [void]$bitmap.UnlockBits($data)
    }
}

$process = if ($ProcessId) {
    Get-Process -Id $ProcessId -ErrorAction Stop
} else {
    Get-Process -Name $Name -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $process) {
    throw "no process named '$Name' is running; start one (see scripts/probe.ps1)"
}
# Correlates with DIASCIA_METRICS elapsed_ms, which counts from the application's own start.
$processStarted = $process.StartTime
$scriptStarted = Get-Date

$hwnd = [IntPtr]::Zero
$search = [System.Diagnostics.Stopwatch]::StartNew()
while ($hwnd -eq [IntPtr]::Zero) {
    $candidates = @(Find-LauncherWindow $process.Id)
    if ($candidates.Count -gt 1) {
        throw "found $($candidates.Count) windows titled '$Title'; close the extra launcher instances"
    }
    if ($candidates.Count -eq 1) {
        $hwnd = $candidates[0]
    } elseif ($search.Elapsed.TotalSeconds -gt 30) {
        throw "no visible '$Title' window from pid $($process.Id) within 30 s; reveal the launcher first"
    } else {
        Start-Sleep -Milliseconds 200
    }
}

$sheet = New-Object System.Drawing.Bitmap(($Patch * 2 + 2), ($Patch * 2 + 2), [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$sheetGraphics = [System.Drawing.Graphics]::FromImage($sheet)
$scratch = New-Object System.Drawing.Bitmap(1, 1, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [System.Drawing.Graphics]::FromImage($scratch)
$grab = { param([int]$x, [int]$y)
    $bitmap = New-Object System.Drawing.Bitmap($Patch, $Patch, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $target = [System.Drawing.Graphics]::FromImage($bitmap)
    $target.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($Patch, $Patch)))
    $target.Dispose()
    $bitmap
}

$clock = [System.Diagnostics.Stopwatch]::StartNew()
$previous = $null
$previousGeometry = ''
$flips = [System.Collections.Generic.List[string]]::new()
$rows = [System.Collections.Generic.List[string]]::new()
New-Item -ItemType Directory -Force -Path $Out | Out-Null

Write-Output "pid $($process.Id)   hwnd $hwnd   '$Title'   sampling $Seconds s at ~$IntervalMs ms"
Write-Output 'Collapse the launcher to an edge, reveal it, then summon it once. This script sends no input.'

$index = 0
while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
    if (-not [DiasciaProbe.Native]::GetWindowRect($hwnd, [ref]$sample)) {
        break
    }
    $geometry = "$($sample.Left),$($sample.Top),$($sample.Right - $sample.Left),$($sample.Bottom - $sample.Top)"
    $moving = if ($previousGeometry -eq $geometry) { 0 } else { 1 }
    $previousGeometry = $geometry

    # tl, tr, bl, br - the same order as the patches drawn into the sheet. Each computed element is
    # parenthesised: inside @() PowerShell binds the comma tighter than the subtraction, so
    # @($a.Right - $n, $a.Top) tries to subtract the whole pair from a number.
    $left = @($sample.Left, $sample.Top)
    $right = @(($sample.Right - $Patch), $sample.Top)
    $belowLeft = @($sample.Left, ($sample.Bottom - $Patch))
    $belowRight = @(($sample.Right - $Patch), ($sample.Bottom - $Patch))
    $origins = @($left, $right, $belowLeft, $belowRight)
    # A hidden launcher keeps its last rect and a minimized window reports -32000; CopyFromScreen
    # returns black for both, which would read as square corners. Those ticks are gaps, not data.
    $box = New-Object System.Drawing.Rectangle($sample.Left, $sample.Top, ($sample.Right - $sample.Left), ($sample.Bottom - $sample.Top))
    $unreadable = ((-not [DiasciaProbe.Native]::IsWindowVisible($hwnd)) -or (-not $virtual.IntersectsWith($box)))
    $runs = @()
    if ($unreadable) {
        $runs = @(-1, -1, -1, -1)
        $previous = $null
    }
    else {
        try {
            for ($c = 0; $c -lt 4; $c++) {
                $bitmap = & $grab $origins[$c][0] $origins[$c][1]
                $runs += Measure-CornerRun $bitmap (($c -band 1) -eq 0) (($c -band 2) -eq 0) $Tolerance
                $sheetGraphics.DrawImage($bitmap, (($c -band 1) * ($Patch + 2)), (($c -shr 1) * ($Patch + 2)), $Patch, $Patch)
                $bitmap.Dispose()
            }
        }
        catch {
            $runs = @(-1, -1, -1, -1)
            $previous = $null
        }
    }

    $wall = [int]$clock.Elapsed.TotalMilliseconds
    $app = [int](($scriptStarted.AddMilliseconds($wall) - $processStarted).TotalMilliseconds)
    $signature = $runs -join '/'
    $rows.Add("$wall,$app,$geometry,$moving,$signature")

    if ($null -ne $previous -and $previous -ne $signature) {
        $file = Join-Path $Out ("{0:d4}-{1}ms-{2}.png" -f $index, $wall, ($signature -replace '/', '-'))
        $sheet.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        $flips.Add("t=${wall}ms app=${app}ms rect=$geometry moving=$moving  $previous -> $signature   $file")
    }
    $previous = $signature
    $index++

    $deadline = $clock.Elapsed.TotalMilliseconds + $IntervalMs
    while ($clock.Elapsed.TotalMilliseconds -lt $deadline) {
        if ($deadline - $clock.Elapsed.TotalMilliseconds -gt 2) {
            Start-Sleep -Milliseconds 1
        } else {
            # Sleep() alone quantises to ~15 ms without timeBeginPeriod, which on a 165 Hz panel
            # would skip every second refresh.
            [System.Threading.Thread]::SpinWait(200)
        }
    }
}
$graphics.Dispose()
$scratch.Dispose()
$sheetGraphics.Dispose()
$sheet.Dispose()

$rows.Insert(0, 'elapsed_ms,app_elapsed_ms,rect,moving,tl,tr,bl,br')
$csvPath = Join-Path (Get-Location) $Csv
[System.IO.File]::WriteAllLines($csvPath, $rows)

Write-Output ''
Write-Output "samples  $index   csv $CsvPath"
Write-Output "pngs     $Out   (one per corner change, tl|tr over bl|br)"
if ($flips.Count -eq 0) {
    Write-Output 'corner flips: none - no diagonal run length changed while sampling.'
} else {
    Write-Output "corner flips: $($flips.Count)"
    $flips | ForEach-Object { Write-Output "  $_" }
}
Write-Output 'reading: each value counts corner-diagonal pixels that are NOT window content. 0 means'
Write-Output '         square (content reaches the extreme pixel); a few means rounded and the size'
Write-Output '         tracks the radius. -1 marks a tick whose patch left the screen. Align'
Write-Output '         app_elapsed_ms with the metrics CSV of the same run.'
