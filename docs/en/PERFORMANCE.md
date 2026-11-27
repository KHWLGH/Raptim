← Back to [README](../../README.en.md) · [简体中文](../PERFORMANCE.md) | **English**

# Performance & Boundaries

This document records Raptim's measured baselines, caching strategies, Win32 edge-frame write findings, and rejected optimizations across storage, icon decoding, event scheduling, and DWM compositing.

- [Storage & Startup Benchmarks](#storage--startup-benchmarks)
- [Icon Pipeline & Memory Budget](#icon-pipeline--memory-budget)
- [Event Loop & Render Scheduling](#event-loop--render-scheduling)
- [Edge Frame Writes & Compositor Boundaries](#edge-frame-writes--compositor-boundaries)
- [Tested & Rejected Optimizations](#tested--rejected-optimizations)

## Storage & Startup Benchmarks

Raptim stores item icons in a dedicated `icon BLOB` column outside `entries.payload` JSON text, encodes JSON exports in Base64, and materializes `dup_key` on write so duplicate validation never has to scan and deserialize the entire table.

Measured before vs. after on the same machine using a **300-item / 7.34 MB synthetic icon fixture** (recorded in commit `764b608`):

| Metric | Before (JSON integer array inside `payload`) | Current (`icon BLOB` + materialized `dup_key`) | Change |
| --- | ---: | ---: | --- |
| `Store::open` full-table read | 187.8 ms | 27.8 ms | ↓ 85.2% |
| Startup to first frame | 294.6 ms | 161.4 ms | ↓ 45.2% |
| Seed/write 300 entries | 33.9 s | 0.35 s | ↓ 99.0% |
| Idle Private memory | 23,970 KB | 19,466 KB | ↓ 18.8% |

On a **300-item / 2.98 MB real-world icon database**, the same design measured **138.8 ms** startup-to-first-frame and **17.2 MB** idle Private memory.

## Icon Pipeline & Memory Budget

- **Downscaled storage (`STORE_MAX = 128`)**: Native icon extraction prioritizes the highest-resolution frame directly from PE (`RT_GROUP_ICON`) and `.ico` resources (including `.lnk` icon locations and embedded non-RGBA PNG frames), falls back to `IShellItemImageFactory` at 256 px (including packaged UWP/MSIX app logos), and finally falls back to `SHGetImageList(SHIL_JUMBO)` with automatic cropping of legacy top-left corner padding on 256 px canvases. Before persistence, file extracts, dropped images, and fetched web icons are downscaled via `Lanczos3` to at most **128 px** PNG on the longest edge (grid icons reach 48 logical pixels, leaving ample headroom for high DPI). Even if a legacy/imported database contains larger PNG payloads, decode downsamples them to 128 px via `Lanczos3`.
- **In-place backfill without full reload**: After background extraction or manual icon refresh, `Store::save_icon` updates only that row's BLOB and swaps the in-memory entry and `IconKey` fingerprint, triggering decode only for visible items instead of reloading the database (cutting per-batch work from O(N × blob) to O(16)).
- **Row virtualization & 24 MiB decode cache**:
  - The main grid and search list request background decode only for the active viewport plus surrounding margin (at most 224 item IDs); scrolled-out items drop their Slint image handles to release pixel buffers.
  - `icons::Cache` maintains an LRU cache covering decoded icons and 36 px drag thumbnails (`ghost`) with a **24 MiB** byte limit (~16 MiB for 224 × 128 px icons + drag thumbnails, plus margin). Offscreen entries are evicted first, and hiding the main window shrinks the cache immediately to the current visible band.

## Event Loop & Render Scheduling

- **Incremental search indexing**: `SearchIndex::sync` checks whether `(name, aliases, notes)` changed before regenerating Pinyin/initial tokens for an entry. Filtered candidate IDs are cached per `(scope, group, collection)` in `eligible` and rebuilt only when collection/item state changes.
- **Signature-diffed model updates**: `ItemSignature` compares visible non-icon fields so unchanged refreshes avoid resetting Slint `VecModel`s.
- **Adaptive wakeups & zero-polling idle**:
  - When Main, Editor, and Settings windows are hidden and protection state is `Normal`, 500 ms periodic checks and interaction timers stop completely (`should_poll_tick` returns `false`), leaving second-instance wakeups to wait on an OS named event with zero polling.
  - While edge-hidden, pointer polling runs at **32 ms** outside the 96 px approach zone and tightens to **16 ms** near the reveal strip. Un-docked focus-loss fade ticks at **8 ms**. Edge slide motion is paced by `FrameClock` aligned to DWM VBlank composition timing and stops as soon as the slide settles.

## Edge Frame Writes & Compositor Boundaries

Investigations (commits `740ea15`, `9483e39`, `54457f0`, `e486eab`) isolated and removed all redundant Win32 frame writes during edge collapse/expand:

1. `decorate()` and `topmost()` previously overwrote `GWL_EXSTYLE` unconditionally, called winit's `set_window_level` (wiping `WS_EX_TOOLWINDOW` and causing taskbar/Alt+Tab flashes), and called `SetWindowRgn(NULL)` repeatedly.
2. All style, region, DWM attribute, and z-order updates now inspect actual live HWND state before writing; `WM_STYLECHANGING` intercepts prevent winit from dropping `WS_EX_TOOLWINDOW`; and `DOCK_OVERHANG_DIP = 10.0` extends the docked edge past the work area so its system corner rounding stays offscreen.

Measured Win32 frame-write counts on live hardware:

| Window state | Collapse writes | Expand writes | Notes |
| --- | ---: | ---: | --- |
| Default z-order | 1 | 1 | Temporary topmost boost at animation start + level restore at animation end |
| "Always on top" enabled | **0** | **0** | Zero style, region, or z-order writes throughout collapse and expand motion |

With "Always on top" producing **0** process-side frame writes during sliding, occasional single-frame corner-mask flicker during fast frame-by-frame window moves is confirmed to originate inside the **Windows DWM compositor** synchronizing rounded corner masks with per-frame window moves. Raptim avoids fighting the compositor and guarantees zero unnecessary non-client frame rebuilds from application code.

## Tested & Rejected Optimizations

The following candidates were benchmarked on the same fixture (commit `764b608`) and intentionally rejected:

- **Raising release optimization to `opt-level = 2`**: Gained ~1.2 ms while bloating binary size by +6.45 MB and degrading single-frame timing; stay on `opt-level = "z"`.
- **Calling `EmptyWorkingSet` when hidden**: Only lowered Task Manager's Working Set display metric (~435 KB) while leaving real Private committed bytes untouched and causing extra page faults on wake.
- **Capping SQLite page cache (`PRAGMA cache_size = -4000`, 4 MiB)**: Because icon BLOBs exceed 4 MiB, shrinking page cache doubled `Store::open` full-table read time (27.8 → 56.5 ms); keep SQLite's ~12.5 MiB default.
- **Installing `SetWindowRgn` clips during animations**: Setting a custom HRGN on a window inside a single visible work area disables DWM anti-aliased corners and creates jagged high-DPI edges; only apply a clip region when multi-monitor docking would bleed across an adjacent display.
