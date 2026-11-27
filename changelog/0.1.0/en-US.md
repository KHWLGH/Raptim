[简体中文](zh-CN.md) | **English**

# 0.1.0 - 2026-11-27

## Added

- Built a native Windows local quick launcher using Rust + Slint 1.18.0 (Fluent style, `winit` backend, software renderer) backed by a single local SQLite `raptim.db` file (WAL mode) beside the executable, with zero account, cloud-sync, or WebView dependencies and `--portable`, `--data-dir <dir>`, and `--silent` CLI flags ([Usage Guide](../../docs/en/USAGE.md)).
- Added two-level group and subcategory organization: horizontal group tabs and left category sidebar each provide adjacent **+** buttons (with no blank-area menus), `F2` inline rename, sibling drag-reordering, cross-group category moves, and guarded deletion with migration or discard options (always keeping at least one group while leaving real files untouched).
- Added item views and interactions: Small (≤ 5 cols), Medium (≤ 4 cols), and Large (≤ 3 cols) icon grids plus List view (auto-narrowing column count); optional hover tab switching (200 / 350 / 600 ms); 180 ms / 12 DIP drag-reorder guard with insertion markers; 6-second coalesced status bar; and item hover tooltips showing full paths and up to 6 lines of notes.
- Added in-window live search and full keyboard workflow: **All / Group / Category** scoping, ranked matching across names, aliases, notes, multiple keywords, and tone-less Pinyin/initials/camelCase with IME composition protection, optional type-to-search, plus `Ctrl+F`, `Ctrl+N`, `Ctrl+,`, `Ctrl+Tab`/`Ctrl+Shift+Tab`, arrow keys/`Home`/`End`, `Enter`/`Shift+Enter`/`Alt+Enter`, `F2`, `Delete`, `Space`, `Ctrl+A`, and layered `Esc`.
- Added multi-source import and maintenance tools: six target types (`Program`, `File`, `Folder`, `URL` with `https://` completion, `Shortcut`, and `App AUMID` picker from `shell:AppsFolder`), Win32 OLE file and browser URL drag-and-drop plus editor custom icon image drop, Start Menu & installed app discovery import, broken item audit with relocate/edit/remove actions, batch selection edit bar, and tri-state tree path conversion.
- Added edge auto-hide and window management: left/right/top edge docking, `DOCK_OVERHANG_DIP` offscreen overhang, critically damped slide animation, 1–12 px physical edge strip width, 150 / 250 / 450 ms collapse delay, focus-loss hide (100 ms delay + 100 ms fade), persisted **Always on top**, system tray menu with session bypasses, global hotkey recorder (`Ctrl+Alt+Space` default), and HKCU startup sync (`--data-dir` + `--silent` with Task Manager status verification).
- Added Game Do Not Disturb (DND) protection and diagnostics: combines up to 100 strict-protected app paths and fullscreen / `SHQueryUserNotificationState` checks to block accidental popups and restore docked strips safely upon release; adds always-on `error-log.txt` (redacted, truncated above 512 KiB) and opt-in `protection-log.csv`.

## Changed

- Completed **Settings → About**: reads `CARGO_PKG_VERSION` automatically, adds GitHub / Issues / Changelog / Copy env info buttons, adds `GPL-3.0-only` card and an offline `Esc`-closable reader (`ListView`-virtualized [LICENSE](../../LICENSE) & third-party notices), plus Rust and `AboutSlint` attribution cards.
- Standardized repository and distribution licensing on `GPL-3.0-only` (matching Slint 1.18.0's GPLv3 path), added root [LICENSE](../../LICENSE) and `Cargo.toml` license metadata, and documented all direct dependencies and bundled assets in [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md).
- Upgraded local storage and JSON format: moved icons from `entries.payload` into a dedicated `icon BLOB` column (Base64 in JSON), materialized `dup_key` on write, split host window geometry and `always_on_top` into `local_state` (excluded from exports), and fail closed without altering bytes on incompatible schemas or unknown fields ([Architecture](../../docs/en/ARCHITECTURE.md#data-storage-and-compatibility-boundaries)).
- Simplified Data management to **Export / Restore** (removing redundant backup snapshots and `rusqlite` `backup` feature); Restore runs inside a single transaction with `dup_key` duplicate pre-checks.
- Unified all in-app context menus into pure-data module `raptim::menus` with bidirectional `region()` verification, reusing blank-area actions in **More** and grouping tray items into four semantic sections.
- Moved name prompt and app picker into in-window overlays, deferred `SettingsWindow` creation until opened, and restored main window focus only when closing foreground child windows.
- Replaced hidden-state polling with per-data-dir hashed OS named events (zero idle polling), adapted edge hover polling between 16 / 32 ms, and drove animations with display-synced `FrameClock`.

## Fixed

- Unified `dup_key` validation across `update_targets_checked` (relocation & path conversion) and all target-mutating code paths while keeping `entries_collection` non-unique so batch swaps and existing duplicate DBs continue to open safely.
- Fixed false relocation prompts on non-path launch errors (such as UAC cancellation) using `Failure.relocatable`.
- Fixed desktop shortcut creation for URLs and Store apps, single-instance listener exits on `WAIT_FAILED`, hotkey cache divergence on `configure` timeout, HTTP `2xx` verification and uppercase `<LINK HREF>` handling in web favicon fetching.
- Eliminated unconditional `GWL_EXSTYLE` writes, `winit::set_window_level` calls (which stripped `WS_EX_TOOLWINDOW`), and redundant `SetWindowRgn` calls causing taskbar flashes and repeated frame rebuilds (confirming 1/1 default and 0/0 always-on-top frame writes; see [Performance & Boundaries](../../docs/en/PERFORMANCE.md#edge-frame-writes--compositor-boundaries)).
- Fixed menu glyphs and reorder boundary checks, and ensured `SHBindToParent`'s interior `ppidlLast` child pointer is never passed to `CoTaskMemFree`.

## Performance & Maintenance

- On a 300-item / 7.34 MB synthetic icon fixture, `Store::open` dropped from 187.8 ms to 27.8 ms, seeding from 33.9 s to 0.35 s, cold startup to first frame from 294.6 ms to 161.4 ms (138.8 ms on a 2.98 MB real-world DB), and idle Private memory from 23,970 KB to 19,466 KB (17.2 MB real-world) ([Performance & Boundaries](../../docs/en/PERFORMANCE.md#storage--startup-benchmarks)).
- Implemented viewport row virtualization, 24 MiB LRU icon decode cache with background workers, and `STORE_MAX = 128` downscaling.
- Modularized binary handlers (`app_*`, `ui_commands`) and Win32 submodules (`platform/*`), consolidating `slint::include_modules!()` inside `src/lib.rs`.
- Replaced fragile pixel/text snapshot checks with pure-logic unit tests and headless `scripts/verify.ps1` verification.
