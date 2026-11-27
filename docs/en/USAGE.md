← Back to [README](../../README.en.md) · [简体中文](../USAGE.md) | **English**

# Usage Guide

This guide covers Raptim's CLI flags, daily interactions, keyboard shortcuts, settings, and data export/restore workflows. For internal design details see [Architecture](ARCHITECTURE.md); for build and verify instructions see [Development & Build](DEVELOPMENT.md).

- [Getting Started and CLI Flags](#getting-started-and-cli-flags)
- [Window Layout & Browsing](#window-layout--browsing)
- [Organizing & Managing Items](#organizing--managing-items)
- [Search & Keyboard Shortcuts](#search--keyboard-shortcuts)
- [Menus & System Tray](#menus--system-tray)
- [Edge Auto-Hide & Window Controls](#edge-auto-hide--window-controls)
- [Game Do Not Disturb (DND)](#game-do-not-disturb-dnd)
- [Settings Reference](#settings-reference)
- [Backups, Export/Restore, and Troubleshooting](#backups-exportrestore-and-troubleshooting)
- [File & Log Formats](#file--log-formats)
- [FAQ](#faq)

## Getting Started and CLI Flags

On first launch, Raptim creates `raptim.db` in the data directory with a starter group and category (**`Common / Default`** on English Windows systems, or **`常用 / 默认分类`** on Simplified Chinese systems). All files stay directly inside the data directory without writing app data under `%AppData%`.

Command-line flags control data location and startup behavior:

```powershell
# Default: read/write raptim.db in the same folder as raptim.exe
.\target\release\raptim.exe

# Portable mode: explicitly target the executable directory
.\target\release\raptim.exe --portable

# Custom data directory: isolate test or multi-profile setups
.\target\release\raptim.exe --data-dir C:\Temp\RaptimTrial

# Silent startup: used for Windows startup or background launch
.\target\release\raptim.exe --silent
```

- `--portable` and `--data-dir` cannot be combined. If the directory is read-only or contains an incompatible database, Raptim shows an error dialog and leaves existing files untouched.
- `--silent` applies on initial startup only: if **Auto-hide at screen edge** is enabled with a saved dock edge, the launcher settles silently at the edge without taking focus; otherwise it stays hidden in the system tray.
- Single-instance per data directory: launching `raptim.exe` again with the same data directory summons the existing instance (subject to Game DND rules) and exits the second process.

## Window Layout & Browsing

The main window uses Fluent styling and Windows 11 rounded corners, keeping rename prompts and management confirmations as in-window overlays:

| Area | Behavior |
| --- | --- |
| Title bar | Drag region on the left; Search, Add item, Pin, **More**, and Close buttons on the right. **Close hides the window** (summon again via hotkey, tray, or relaunch); there is no minimize button. |
| Top group bar | Horizontally scrollable top-level groups, plus a trailing **+** button to create a group (no background context menu). |
| Left category sidebar | Subcategories under the selected group with wrapping labels, vertical scrolling, and a bottom **+** button (no background context menu). |
| Item view | Grid view (Small 28 px ≤ 5 cols / Medium 36 px ≤ 4 cols / Large 48 px ≤ 3 cols, dropping to ≥ 2 cols on narrow windows) and single-column List view. Clicking shows focus outlines during keyboard navigation without sticky highlight boxes. |
| Hover tooltip & status bar | Hovering reveals name, full target path/URL, and up to 6 lines of notes. Bottom status messages auto-dismiss after 6 seconds and coalesce identical messages. |

Empty categories offer direct **Add item** and **Scan & import** buttons plus drag-and-drop hints.

## Organizing & Managing Items

### Item Types & Editor

Click **+** or press `Ctrl+N` to open the editor window:

| Type | Notes & Dedicated Fields |
| --- | --- |
| `Program` | Local executable with optional arguments, working directory, **Run as administrator**, and Run mode (`Normal` / `Maximized` / `Minimized`) |
| `File` | Any local or UNC file path |
| `Folder` | Local or network folder |
| `URL` | Scheme-less domains (e.g. `example.com`) auto-complete to `https://example.com`; only `http://` and `https://` allowed |
| `Shortcut` | `.lnk` shortcut path with Run mode (`Normal` / `Maximized` / `Minimized`) |
| `App AUMID` | Opens the in-window `shell:AppsFolder` app picker to select installed Store/UWP apps and extract their icons |

- **Category picker**: Keyboard-friendly group/category tree (`Enter` to select, `Esc` closes the tree overlay without losing form edits).
- **Icons & image drop**: Extracts up to 256 px Shell icons and downscales to 128 px PNG. Web URL favicons are fetched on save, external drop, or manual **Refresh icon** (startup icon backfill never connects to the internet). Drag any PNG, JPEG, or ICO image (up to 16 MB, 4096×4096) onto the editor to use a custom icon.
- **Unsaved changes guard**: Prompts before discarding modified fields; `Ctrl+Enter` saves immediately.

### Drag-and-Drop & Bulk Import

- **Explorer / Browser drop**: Drop up to 500 local files/folders/shortcuts or browser URLs at once. Dropping onto a sidebar category imports directly into that category; duplicates within the destination category are skipped automatically.
- **Scan & import apps**: Open from **More → Scan & import apps…**, blank-space context menu, or the empty-category card. Scans Start Menu shortcuts (up to 4 levels deep, filtering uninstallers/readme/help entries) and `AppsFolder` packaged apps, lets you filter by keyword, and imports checked items excluding existing duplicates.

### Reordering, Collections & Batch Mode

- **Drag guard**: Holding left mouse button for **≥ 180 ms** and moving **≥ 12 DIP** starts drag-reordering with half-card/half-row insertion lines, preventing accidental triggers on quick flicks.
- **Groups & categories**: Support drag-reordering among siblings, `F2` inline rename, **Move to group** for categories, and safe **Move & delete** or **Delete items without moving** (at least one top-level group is always retained).
- **Batch selection**: Open **Batch edit** from **More** or blank-space menu; click, `Shift+Click`, `Space`, and `Ctrl+A` to select items, then use **Move to…**, **Copy to…**, or **Delete** (removes launcher records only; original files remain intact). Duplicate clashes cancel the batch atomically without partial changes.

### Path Conversion & Broken Item Audit

- **Batch path conversion**: **More → Convert paths** converts program paths and working directories relative to `raptim.db` (or back to absolute) with a tri-state scope tree and preview. Cross-drive targets are skipped cleanly.
- **Check broken items**: **More → Data → Check broken items…** audits local paths, `PATH` executables, and installed AUMIDs (skipping URLs), offering **Relocate**, **Edit**, and **Remove** actions per broken entry.

## Search & Keyboard Shortcuts

Click the search icon or press `Ctrl+F` to open the live search overlay. When **Type to search when focused** is enabled in General settings, typing any printable character while the main window has focus opens search automatically.

- **Scope**: Filter by **All**, **Group**, or **Category**.
- **Matching**: Matches item names, aliases (split by spaces, commas, semicolons, `、`, or `|`), and notes (**target paths excluded**) with multi-keyword, Pinyin, initials, and camelCase scoring. Pressing `Enter` during IME composition commits composition text instead of launching an item.

| Shortcut | Scope | Action |
| --- | --- | --- |
| `Ctrl+Alt+Space` (default) | Global | Summon main window, or collapse/hide if already expanded and focused |
| `Ctrl+F` | Main window | Open and focus search bar |
| `Ctrl+N` | Main window | Open Add item window |
| `Ctrl+,` | Main window | Open Settings window |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Main window | Switch to next / previous category |
| `↑` `↓` `←` `→` / `Home` `End` | Main window | Navigate items or search results |
| `Enter` | Item / Search result | Launch item (or toggle checkbox in batch mode) |
| `Shift+Enter` | Item / Search result | Run as administrator |
| `Alt+Enter` | Item / Search result | Open Editor for selected item |
| `F2` | Nav / Item | Inline-rename focused group/category, or open Editor for focused item |
| `Delete` | Item grid | Prompt to delete selected item |
| `Space` | Batch mode | Toggle selection on focused item |
| `Ctrl+A` | Batch mode | Select all items in current view |
| `Esc` | Main window (layered) | Dismisses top-most overlay step-by-step down to hiding the window |
| `Ctrl+Enter` / `Esc` | Editor | Save item / close (prompts if dirty) |

## Menus & System Tray

All context menus are owner-drawn in Light/Dark theme with icons and semantic separators:

- **Title-bar "More" & blank-area context menu**: Shares batch edit, add item/files, scan & import, icon size, list view, and sort by name; **More** adds **Data** (Export, Restore, Check broken items, Open data folder), **Convert paths**, **Reset window position**, **Settings…**, and **Exit**.
- **Item context menu**: Open / Open file location / Copy target → Edit / Relocate → Run as administrator / Windows native menu / Create desktop shortcut / Refresh icon → Move to / Copy to → Reorder / Path conversion → Delete. Unsupported actions (like elevation or path conversion on URLs) and out-of-bounds moves are disabled automatically.
- **System tray menu**: Right-click exposes **Show Raptim** / **Reset window position**, session toggles (**Pause auto-hide**, **Pause Game DND** synced bi-directionally with pin & settings), **Settings…**, and **Exit**. Left-clicking the tray icon shows the window.

## Edge Auto-Hide & Window Controls

With **Auto-hide at screen edge** enabled, dragging the main window within ~14 DIP of the left, right, or top screen edge snaps it there:

- **Hover expand & collapse delay**: Resting the pointer on the edge strip for **24 ms** expands the window; moving out waits **150 / 250 / 450 ms** (default 250 ms) before sliding shut. Summoning via hotkey, tray, or second instance keeps the window open until pointer/focus leaves so it doesn't collapse immediately.
- **Edge strip & cloak**: Configure strip visibility and **1–12 physical pixels** width (default 1 px). Disabling **Show edge strip** cloaks the hidden window with DWM while still responding to hover at the edge or hotkeys.
- **Auxiliary & focus protection**: Opening Settings, Editor, inline rename, or file dialogs pauses focus-loss hide, edge collapse, and normal fullscreen hiding, returning focus cleanly when an active child window closes.
- **Always on top vs. Pin**: Title-bar **Pin** temporarily keeps the window visible without saving; **Always on top** in Settings persists across restarts and coexists with edge docking.
- **Reset position**: Recovers an offscreen window (e.g., after disconnecting a monitor) centered inside the nearest monitor's work area.

## Game Do Not Disturb (DND)

- **Fullscreen protection** (enabled by default): Combines fullscreen window geometry with `SHQueryUserNotificationState` (`D3D` fullscreen, presentation mode, busy) to ignore accidental hotkeys, tray clicks, or edge hovers. Releases after a short debounce (900 ms normal, 3000 ms when `QUNS_BUSY`).
- **Strict protected apps**: Add up to 100 executable paths in **Settings → Game DND**. Matching foreground processes override temporary pin and auxiliary windows and block global hotkey response.
- **Session pause & diagnostics**: Check **Pause Game DND** in tray/settings for the current run only, or enable **Log protection decisions** to inspect redacted entries in `protection-log.csv`.

## Settings Reference

| Tab | Setting | Description & Default |
| --- | --- | --- |
| **General** | Theme | `Follow system` (default) / `Light` / `Dark` |
| | Language | `Follow system` (default, matches Windows UI language) / `简体中文` / `English`; applies live without restarting |
| | Launch at startup | Writes HKCU `CurrentVersion\Run` with `--data-dir` & `--silent` (off by default); warns if path moved or disabled in Task Manager |
| | Hide after launch | Automatically hides main window after launching an item (off) |
| | Launch click | `Single click` (default) / `Double click` |
| | Type to search when focused | Printable keystrokes open live search immediately (off) |
| | Switch categories on hover / Hover delay | Auto-select hovered tabs after `200 ms` / `350 ms` (default) / `600 ms` (off) |
| **Summon & Hide** | Global hotkey | Record shortcut or reset to `Ctrl+Alt+Space` |
| | Auto-hide at screen edge / Show strip / Width | Enabled / Enabled / `1` physical px (`1–12`) |
| | Edge collapse delay | `150 ms` / `250 ms` (default) / `450 ms` |
| | Always on top | Persisted per machine in `local_state` (off) |
| | Hide on focus loss | Wait 100 ms + 100 ms fade out when undocked (off) |
| | Window position off-screen? | **Reset position** centers the window on the nearest display |
| **Game DND** | Fullscreen protection | Enabled by default |
| | Pause Game DND | Session-only bypass toggle |
| | Log protection decisions | Writes path-redacted `protection-log.csv` (off) |
| | Strict protected apps | Up to 100 full executable paths |
| **Data** | Data directory | Displays active database folder |
| | Prefer relative paths for new items | Stores paths relative to the data folder for portable setups (off) |
| | Data management | **Export**, **Restore**, **Open data folder**, **View error log** (shown when log exists) |
| **About** | Version, links & legal | Shows version/toolchain/engine/storage, GitHub/Issues/Changelog/Copy env info, GPL-3.0-only & third-party notices viewer, Rust & `AboutSlint` links |

## Backups, Export/Restore, and Troubleshooting

- **Full folder backup**: Exit Raptim completely before copying your data folder (`raptim.db` + `raptim.db-wal` + `raptim.db-shm`) so WAL frames are preserved.
- **JSON Export & Restore**: Exports groups, entries, Base64 icons, and portable settings (`language`, `theme_mode`, `hotkey`, etc.) while keeping host-specific `local_state` separate. Restore runs a strict pre-flight structure & duplicate check, then replaces database contents in one atomic transaction.
- **Launch relocation**: Missing file/folder/shortcut targets, offline drives, or unreachable network paths open the in-window **Relocate** dialog, updating only the path while preserving arguments, working dir, and run mode.
- **Error log**: `error-log.txt` records launch errors, unrouted commands, and panics with redacted user/data paths and auto-truncates above 512 KiB.

## File & Log Formats

### JSON Export (`format_version = 2`)

```json
{
  "format_version": 2,
  "collections": [
    { "id": 1, "parent_id": null, "name": "Common", "position": 0 },
    { "id": 2, "parent_id": 1, "name": "Default", "position": 0 }
  ],
  "entries": [
    {
      "icon": "iVBORw0KGgo...",
      "id": 1,
      "collection_id": 2,
      "name": "Notepad",
      "aliases": "notepad text",
      "notes": "Quick text editor",
      "target": {
        "kind": "program",
        "path": "C:\\Windows\\System32\\notepad.exe",
        "arguments": "",
        "working_dir": "",
        "elevated": false,
        "show": "normal"
      },
      "position": 0
    }
  ],
  "settings": {
    "hotkey": "Ctrl+Alt+Space",
    "theme_mode": 0,
    "language": 0,
    "edge_hide": true
  }
}
```

### Diagnostic Logs

- `error-log.txt`: `<unix_ms> <category> <redacted_message>`
- `protection-log.csv`: CSV with header `elapsed,kind,source,protection,detail`

## FAQ

- **Why didn't clicking the top-right × close the process?** As a resident launcher, `×` hides the window cleanly. Use **Tray → Exit** or **More → Exit** to quit.
- **Why isn't the hotkey responding during a game?** Either another app claimed the hotkey (check Settings warning) or Game DND blocked it; use **Pause Game DND** in the tray for the current session or enable **Log protection decisions** to diagnose.
- **Window lost after unplugging an external monitor?** Click **Reset position** in Tray, More, or Settings → Summon & Hide.
- **Startup red warning in Settings?** Your `.exe` moved, `--data-dir` changed, or startup was disabled in Windows Task Manager. Toggle **Launch at startup** off and on again (or re-enable in Task Manager).
- **Web URL added without an icon on restart?** To keep startup offline-first, startup backfill skips URLs; right-click the item and choose **Refresh icon**.
