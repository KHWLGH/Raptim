<#
.SYNOPSIS
    Watches the four corners of the launcher's native window while a human slides it. Sends no input.

.DESCRIPTION
    Why this exists: DWM rounding cannot be proven offscreen (see the verification layers in README),
    and the metrics CSV already showed the work-area HRGN path never runs on a single display. So the
    corner/shadow pop has to be watched on the composited desktop.

    It samples the screen, never the window: PrintWindow and GetDC(hwnd) both return the client
    surface, which has no corners, because DWM applies the mask and draws the shadow in the
    compositor.

    HOW A CORNER IS JUDGED - and why the first version of this script could not be trusted. Measuring
    the extreme pixel against a reference taken inside the same patch reports "square" whenever the
    desktop behind the corner happens to match the window surface, which is exactly what a hover
    repaint in that region causes. Each corner is therefore judged *differentially*: the extreme pixel
    is compared to the band just OUTSIDE the window rect (what the compositor shows there when the mask
    is on) and to the content well INSIDE the rect, and the corner counts as rounded when it looks like
    the former. The decision margin is recorded too, so a reading can be distrusted on purpose. That
    outside band doubles as the shadow measurement, sampled every tick: it jumping while the window is
    not moving is a shadow change with the mask untouched.

    A crop is only read when it fits entirely inside the monitor the window is on, and a corner is
    only judged when the two references differ from each other by at least -Spread. Both guards exist
    because of what the first runs on a multi-monitor desktop showed: a patch reaching onto the
    neighbouring display, or reaching off the desktop at all, comes back as that other content or as
    plain black - and both look exactly like a squared corner. IntersectsWith is not enough either;
    one of 72 columns on screen is the collapsed state and says nothing.

    WHAT TO DO: start it, then do one collapse, one reveal, and one hotkey summon. Read the event
    lines: mask flips while moving=1 point at the per-frame window moves; mask flips while moving=0
    point at a call that rebuilt the frame at rest; band jumps without a mask flip are shadow-only, and
    the active / cursor columns say whether activation or a hover repaint explains them.

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
    [int]$Seconds = 15,
    [int]$IntervalMs = 5,
    [int]$Crop = 72,
    [int]$Outside = 24,
    [int]$Band = 8,
    [int]$Depth = 26,
    [int]$BandJump = 14,
    [int]$Spread = 90,
    [int]$MinMargin = 60,
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
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X; public int Y; }
    public static class Native {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
        [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr hWnd, uint flags);
        [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr hMonitor, ref MONITORINFO info);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int maxCount);
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct MONITORINFO {
        public uint cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public uint dwFlags;
    }
}
'@)

# Without this every coordinate below is virtualised by the shell and the samples come from the wrong
# part of the screen.
if (-not [DiasciaProbe.Native]::SetProcessDpiAwarenessContext([IntPtr](-4))) {
    [void][DiasciaProbe.Native]::SetProcessDPIAware()
}

# SM_X/SM_Y/SM_CX/SM_CY VIRTUALSCREEN: the union of every display, negative origins included. That is
# the only area a crop can honestly be read from.
$virtual = New-Object System.Drawing.Rectangle(
    [DiasciaProbe.Native]::GetSystemMetrics(76),
    [DiasciaProbe.Native]::GetSystemMetrics(77),
    [DiasciaProbe.Native]::GetSystemMetrics(78),
    [DiasciaProbe.Native]::GetSystemMetrics(79))
$sample = New-Object DiasciaProbe.RECT
$cursor = New-Object DiasciaProbe.POINT

function Find-LauncherWindow([int]$targetPid) {
    # Settings and editor windows belong to the same process, and the launcher also owns a 26x26
    # taskbar-restore window. Title plus area picks the one window this script may read.
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

function Get-Red([int]$color) { $color -band 0xff }
function Get-Green([int]$color) { ($color -shr 8) -band 0xff }
function Get-Blue([int]$color) { ($color -shr 16) -band 0xff }

function Get-Distance([int]$one, [int]$other) {
    [math]::Abs((Get-Red $one) - (Get-Red $other)) +
    [math]::Abs((Get-Green $one) - (Get-Green $other)) +
    [math]::Abs((Get-Blue $one) - (Get-Blue $other))
}

function Get-Average($data, [int]$x, [int]$y, [int]$stepX, [int]$stepY, [int]$from, [int]$to, [bool]$luma) {
    # Walks the corner diagonal, in pixels $from..$to away from the extreme pixel, and returns either a
    # packed RGB or its luminance. $from starts at 2 so the extreme pixel itself never pollutes the
    # reference it is compared against.
    $r = 0.0
    $g = 0.0
    $b = 0.0
    $count = 0
    for ($k = $from; $k -le $to; $k++) {
        $p = Read-Pixel $data ($x + $stepX * $k) ($y + $stepY * $k)
        $r += (Get-Red $p)
        $g += (Get-Green $p)
        $b += (Get-Blue $p)
        $count++
    }
    if ($luma) {
        return [int](($r + $g + $b) / (3 * $count))
    }
    return [int]($r / $count) -bor ([int]($g / $count) -shl 8) -bor ([int]($b / $count) -shl 16)
}

function Measure-Corner($data, [int]$x, [int]$y, [int]$signX, [int]$signY, [int]$band, [int]$depth, [int]$spread) {
    # rounded(1)/square(0)/no-opinion(-1), the decision margin, and the shadow band luminance. Outside
    # the window rect is $sign; content is the opposite direction.
    $extreme = Read-Pixel $data $x $y
    $outside = Get-Average $data $x $y $signX $signY 2 $band $false
    $inside = Get-Average $data $x $y (-$signX) (-$signY) 20 $depth $false
    # If the two references are not distinguishable from each other, no comparison of the corner
    # pixel against them can say anything - report that instead of guessing.
    if ((Get-Distance $outside $inside) -lt $spread) {
        return @(-1, -1, (Get-Average $data $x $y $signX $signY 2 $band $true))
    }
    $toOutside = Get-Distance $extreme $outside
    $toInside = Get-Distance $extreme $inside
    $shadow = Get-Average $data $x $y $signX $signY 2 $band $true
    $rounded = if ($toOutside -le $toInside) { 1 } else { 0 }
    return @($rounded, [math]::Abs($toInside - $toOutside), $shadow)
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

$sheet = New-Object System.Drawing.Bitmap(($Crop * 2 + 2), ($Crop * 2 + 2), [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$sheetGraphics = [System.Drawing.Graphics]::FromImage($sheet)
$bounds = New-Object System.Drawing.Rectangle(0, 0, $Crop, $Crop)
# tl, tr, bl, br. The crop is positioned so the window rect's extreme pixel always lands at the index
# used below: $Outside pixels of desktop on the outward side, the rest inside the window.
$signsX = @(-1, 1, -1, 1)
$signsY = @(-1, -1, 1, 1)

$clock = [System.Diagnostics.Stopwatch]::StartNew()
$previousStates = $null
$previousShadows = $null
$previousGeometry = ''
$events = [System.Collections.Generic.List[string]]::new()
$rows = [System.Collections.Generic.List[string]]::new()
$unreadable = 0
$unsure = 0
New-Item -ItemType Directory -Force -Path $Out | Out-Null

Write-Output "pid $($process.Id)   hwnd $hwnd   '$Title'   sampling $Seconds s at ~$IntervalMs ms"
Write-Output 'Collapse the launcher to an edge, reveal it, then summon it once. This script sends no input.'

$index = 0
while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
    if (-not [DiasciaProbe.Native]::GetWindowRect($hwnd, [ref]$sample)) {
        break
    }
    $geometry = "$($sample.Left),$($sample.Top),$($sample.Right),$($sample.Bottom)"
    $moving = if ($previousGeometry -eq $geometry) { 0 } else { 1 }
    $previousGeometry = $geometry
    [void][DiasciaProbe.Native]::GetCursorPos([ref]$cursor)
    $pointerInside = if ($cursor.X -ge $sample.Left -and $cursor.X -lt $sample.Right -and
        $cursor.Y -ge $sample.Top -and $cursor.Y -lt $sample.Bottom) { 1 } else { 0 }
    $active = if ([DiasciaProbe.Native]::GetForegroundWindow() -eq $hwnd) { 1 } else { 0 }
    $shown = [DiasciaProbe.Native]::IsWindowVisible($hwnd)
    # A crop may only be read from the monitor the window itself is on: a patch reaching onto the
    # neighbouring display compares the launcher against that display's content, and one reaching off
    # the desktop at all comes back black - both look exactly like a squared corner. Containment, not
    # intersection: 1 of 72 columns on screen is the collapsed state and says nothing.
    $area = $virtual
    $monitor = New-Object DiasciaProbe.MONITORINFO
    $monitor.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($monitor)
    $fromWindow = [DiasciaProbe.Native]::MonitorFromWindow($hwnd, 2)
    if ($fromWindow -ne [IntPtr]::Zero -and
        [DiasciaProbe.Native]::GetMonitorInfo($fromWindow, [ref]$monitor)) {
        $edge = $monitor.rcMonitor
        $area = New-Object System.Drawing.Rectangle($edge.Left, $edge.Top,
            ($edge.Right - $edge.Left), ($edge.Bottom - $edge.Top))
    }

    $states = @(0, 0, 0, 0)
    $margins = @(0, 0, 0, 0)
    $shadows = @(-1, -1, -1, -1)
    for ($c = 0; $c -lt 4; $c++) {
        $states[$c] = -1
        $margins[$c] = -1
        $signX = $signsX[$c]
        $signY = $signsY[$c]
        $cornerX = $(if ($signX -lt 0) { $sample.Left } else { $sample.Right - 1 })
        $cornerY = $(if ($signY -lt 0) { $sample.Top } else { $sample.Bottom - 1 })
        $originX = $cornerX - $(if ($signX -lt 0) { $Outside } else { $Crop - $Outside - 1 })
        $originY = $cornerY - $(if ($signY -lt 0) { $Outside } else { $Crop - $Outside - 1 })
        $box = New-Object System.Drawing.Rectangle($originX, $originY, $Crop, $Crop)
        if ((-not $shown) -or (-not $area.Contains($box))) {
            # Nothing honest can be said about this corner this tick.
            $unreadable++
            continue
        }
        $bitmap = New-Object System.Drawing.Bitmap($Crop, $Crop, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $target = [System.Drawing.Graphics]::FromImage($bitmap)
        $target.CopyFromScreen($originX, $originY, 0, 0, (New-Object System.Drawing.Size($Crop, $Crop)))
        $target.Dispose()
        $sheetGraphics.DrawImage($bitmap, (($c -band 1) * ($Crop + 2)), (($c -shr 1) * ($Crop + 2)), $Crop, $Crop)
        $data = $bitmap.LockBits($bounds, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
            [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $measured = Measure-Corner $data ($cornerX - $originX) ($cornerY - $originY) $signX $signY $Band $Depth $Spread
        }
        finally {
            [void]$bitmap.UnlockBits($data)
            $bitmap.Dispose()
        }
        $states[$c] = $measured[0]
        $margins[$c] = $measured[1]
        $shadows[$c] = $measured[2]
        if ($measured[1] -lt $MinMargin) {
            # About equally far from both references: recorded, deliberately not believed.
            $unsure++
        }
    }

    $wall = [int]$clock.Elapsed.TotalMilliseconds
    $app = [int](($scriptStarted.AddMilliseconds($wall) - $processStarted).TotalMilliseconds)
    $line = @($wall, $app, $sample.Left, $sample.Top, ($sample.Right - $sample.Left),
        ($sample.Bottom - $sample.Top), $moving, $pointerInside, $active) + $states + $margins + $shadows
    $rows.Add(($line -join ','))
    $stateText = $states -join '/'

    # A change is only an event when both sides of it could actually be read; otherwise the -1s of a
    # collapsed window would generate a flip on every entry and exit of the screen edge.
    $readable = ($states -notcontains -1) -and ($null -ne $previousStates) -and
    (-not ($previousStates.Split('/') -contains '-1'))
    $reason = $null
    if ($readable -and $previousStates -ne $stateText) {
        $reason = 'mask'
    }
    elseif ($null -ne $previousShadows) {
        for ($c = 0; $c -lt 4; $c++) {
            if ($shadows[$c] -ge 0 -and $previousShadows[$c] -ge 0 -and
                [math]::Abs($shadows[$c] - $previousShadows[$c]) -gt $BandJump) {
                $reason = "band$c"
                break
            }
        }
    }
    if ($null -ne $reason) {
        $name = ('{0:d4}-{1}ms-{2}-{3}-{4}-{5}.png' -f $index, $wall, $reason, ($states -join ''),
            ($shadows -join ''), $moving)
        $file = Join-Path $Out $name
        $sheet.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        $events.Add("$reason t=${wall}ms app=${app}ms rect=$geometry moving=$moving cursor=$pointerInside active=$active states $previousStates -> $stateText margins $($margins -join '/') shadow $($shadows -join '/') $(Split-Path $file -Leaf)")
    }
    $previousStates = $stateText
    $previousShadows = $shadows
    $index++

    $deadline = $clock.Elapsed.TotalMilliseconds + $IntervalMs
    while ($clock.Elapsed.TotalMilliseconds -lt $deadline) {
        if ($deadline - $clock.Elapsed.TotalMilliseconds -gt 2) {
            Start-Sleep -Milliseconds 1
        } else {
            # Sleep() alone quantises to ~15 ms without timeBeginPeriod, which on a 165 Hz panel would
            # skip every second refresh.
            [System.Threading.Thread]::SpinWait(200)
        }
    }
}
$sheetGraphics.Dispose()
$sheet.Dispose()

$rows.Insert(0, ('elapsed_ms,app_elapsed_ms,x,y,w,h,moving,cursor_inside,active,tl,tr,bl,br,' +
    'margin_tl,margin_tr,margin_bl,margin_br,shadow_tl,shadow_tr,shadow_bl,shadow_br'))
$csvPath = Join-Path (Get-Location) $Csv
[System.IO.File]::WriteAllLines($csvPath, $rows)

$maskEvents = @($events | Where-Object { $_ -like 'mask *' })
$bandEvents = @($events | Where-Object { $_ -notlike 'mask *' })
Write-Output ''
Write-Output "samples        $index   csv $CsvPath"
Write-Output "crops          $Out   (2x2 sheet: tl|tr over bl|br, $Crop px each)"
Write-Output "unreadable     $unreadable corner ticks off the desktop (-1); unsure $unsure (margin < $MinMargin)"
Write-Output "mask flips     $($maskEvents.Count)"
$maskEvents | ForEach-Object { Write-Output "  $_" }
Write-Output "shadow jumps   $($bandEvents.Count) (outside band moved by more than $BandJump, mask unchanged)"
$bandEvents | Select-Object -First 24 | ForEach-Object { Write-Output "  $_" }
Write-Output 'reading: 1 = the extreme pixel looks like the desktop outside the rect, so the mask is on;'
Write-Output '         0 = it looks like window content, so the corner is square; -1 = no reading.'
Write-Output '         mask flips at moving=0 are a frame rebuild at rest; band-only jumps at moving=0'
Write-Output '         are the shadow, and active/cursor say whether activation or a hover repaint did it.'
Write-Output '         Align app_elapsed_ms with the metrics CSV of the same run.'
