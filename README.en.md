<div align="center">

<img src="ui/icons/raptim.svg" width="96" alt="Raptim" />

# Raptim

**A lightweight local launcher for Windows**

[简体中文](README.md) | **English**

[![License](https://img.shields.io/badge/license-GPL--3.0--only-blue?style=flat-square)](LICENSE)
[![Stars](https://img.shields.io/github/stars/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/stargazers)
[![Downloads](https://img.shields.io/github/downloads/KHWLGH/Raptim/total?style=flat-square)](https://github.com/KHWLGH/Raptim/releases)
[![Issues](https://img.shields.io/github/issues/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/issues)
[![Last commit](https://img.shields.io/github/last-commit/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/commits/main)
[![Repo size](https://img.shields.io/github/repo-size/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim)
[![Top language](https://img.shields.io/github/languages/top/KHWLGH/Raptim?style=flat-square&color=CE422B)](https://github.com/KHWLGH/Raptim)

[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011%20x64-0078D4?style=flat-square&logo=windows11&logoColor=white)](#quick-start)
[![Rust](https://img.shields.io/badge/Rust-1.92%2B%20%7C%20Edition%202024-CE422B?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Slint](https://img.shields.io/badge/UI-Slint%201.18.0%20Fluent-2379F4?style=flat-square)](https://slint.dev/)
[![SQLite](https://img.shields.io/badge/storage-SQLite%20WAL-003B57?style=flat-square&logo=sqlite&logoColor=white)](docs/en/ARCHITECTURE.md#data-storage-and-compatibility-boundaries)
[![Native](https://img.shields.io/badge/native-Win32%20%7C%20No%20WebView-4B5563?style=flat-square)](docs/en/ARCHITECTURE.md)
[![Portable](https://img.shields.io/badge/mode-Portable%20%7C%20Offline--first-10B981?style=flat-square)](docs/en/USAGE.md#getting-started-and-cli-flags)

</div>

## Overview

Raptim gathers your frequently used programs, files, folders, URLs, and installed Windows Store apps into a single compact launcher. It docks cleanly to a screen edge and hides out of the way until summoned by a global hotkey or a hover at the edge, then lets you launch anything by name, alias, or Pinyin.

Built natively with **Rust**, **Slint**, and embedded **SQLite**, Raptim requires no WebView or external runtime and never performs background whole-disk indexing. All settings, groups, entries, and cached icons live offline in a local database directory; the only network requests happen when you add or refresh a web URL icon (or click external links in About).

> Supports both **English** and **Simplified Chinese (`简体中文`)**, automatically matching your Windows UI language out of the box with live switching in **Settings → General → Language**.

## UI Previews

<p align="center">
  <img src="docs/previews/en/overview-light.png#gh-light-mode-only" alt="Raptim English UI Overview" width="100%" />
  <img src="docs/previews/en/overview-dark.png#gh-dark-mode-only" alt="Raptim English UI Overview" width="100%" />
</p>

<p align="center">
  <sub>Main grid, live search overlay, item editor, and settings window (automatically switches with GitHub's Light / Dark theme)</sub>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Main Window · Grid View</strong><br />
      <sub>Top group tabs, category sidebar, and multi-column icon grid</sub><br /><br />
      <img src="docs/previews/en/light/main-grid.png#gh-light-mode-only" alt="Main window grid view" width="320" />
      <img src="docs/previews/en/dark/main-grid.png#gh-dark-mode-only" alt="Main window grid view" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>Main Window · Live Search</strong><br />
      <sub>All / Group / Category scopes with multi-keyword and Pinyin matching</sub><br /><br />
      <img src="docs/previews/en/light/main-search.png#gh-light-mode-only" alt="Main window live search" width="320" />
      <img src="docs/previews/en/dark/main-search.png#gh-dark-mode-only" alt="Main window live search" width="320" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Scan & Import Installed Apps</strong><br />
      <sub>Background scan of Start Menu and Store/UWP apps with batch import</sub><br /><br />
      <img src="docs/previews/en/light/main-discover.png#gh-light-mode-only" alt="Scan and import installed apps" width="320" />
      <img src="docs/previews/en/dark/main-discover.png#gh-dark-mode-only" alt="Scan and import installed apps" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>Item Editor</strong><br />
      <sub>Programs, files, folders, URLs, shortcuts, and App AUMIDs</sub><br /><br />
      <img src="docs/previews/en/light/editor-program.png#gh-light-mode-only" alt="Item editor window" width="360" />
      <img src="docs/previews/en/dark/editor-program.png#gh-dark-mode-only" alt="Item editor window" width="360" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Settings · Summon & Hide</strong><br />
      <sub>Global hotkey recorder, edge auto-hide, strip width, and always on top</sub><br /><br />
      <img src="docs/previews/en/light/settings-summon.png#gh-light-mode-only" alt="Settings Summon and Hide" width="420" />
      <img src="docs/previews/en/dark/settings-summon.png#gh-dark-mode-only" alt="Settings Summon and Hide" width="420" />
    </td>
    <td width="50%" align="center">
      <strong>Settings · Game DND Protection</strong><br />
      <sub>Fullscreen protection, session bypass, and strict protected app list</sub><br /><br />
      <img src="docs/previews/en/light/settings-protection.png#gh-light-mode-only" alt="Settings Game DND" width="420" />
      <img src="docs/previews/en/dark/settings-protection.png#gh-dark-mode-only" alt="Settings Game DND" width="420" />
    </td>
  </tr>
</table>

<details>
<summary>View More UI Screenshots (List View, Batch Actions, Broken Check & Settings)</summary>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>Main Window · List View</strong><br />
      <sub>Compact list layout showing item names and target paths</sub><br /><br />
      <img src="docs/previews/en/light/main-list.png#gh-light-mode-only" alt="Main window list view" width="320" />
      <img src="docs/previews/en/dark/main-list.png#gh-dark-mode-only" alt="Main window list view" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>Batch Multi-Select</strong><br />
      <sub>Select multiple items to move, copy, or delete from the floating action bar</sub><br /><br />
      <img src="docs/previews/en/light/main-batch.png#gh-light-mode-only" alt="Batch multi-select" width="320" />
      <img src="docs/previews/en/dark/main-batch.png#gh-dark-mode-only" alt="Batch multi-select" width="320" />
    </td>
  </tr>
  <tr>
    <td colspan="2" align="center">
      <strong>Broken Item Check</strong><br />
      <sub>Offline scan for missing files, folders, and uninstalled apps with quick relocate or remove</sub><br /><br />
      <img src="docs/previews/en/light/main-broken.png#gh-light-mode-only" alt="Broken item check" width="320" />
      <img src="docs/previews/en/dark/main-broken.png#gh-dark-mode-only" alt="Broken item check" width="320" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>Settings · General</strong><br />
      <sub>Theme, bilingual UI language, autostart, click mode, and hover-to-switch delay</sub><br /><br />
      <img src="docs/previews/en/light/settings-general.png#gh-light-mode-only" alt="Settings General" width="420" />
      <img src="docs/previews/en/dark/settings-general.png#gh-dark-mode-only" alt="Settings General" width="420" />
    </td>
    <td width="50%" align="center">
      <strong>Settings · About</strong><br />
      <sub>Version info, one-click diagnostics copy, GPLv3 license, and third-party attributions</sub><br /><br />
      <img src="docs/previews/en/light/settings-about.png#gh-light-mode-only" alt="Settings About" width="420" />
      <img src="docs/previews/en/dark/settings-about.png#gh-dark-mode-only" alt="Settings About" width="420" />
    </td>
  </tr>
</table>

</details>

> To render all 21 UI scenes locally without stealing focus (including batch actions, path conversion, broken item checks, and the standalone HTML gallery), run `powershell -File scripts/preview.ps1 -Preset all -Lang en`. See [Headless UI Preview Generator](docs/en/DEVELOPMENT.md#headless-ui-preview-generator).

## Tech Stack & Design Rationale

| Layer | Choice | Rationale |
| --- | --- | --- |
| Language & build | Rust 1.92+ (Edition 2024) | Ahead-of-time native compilation with no GC pauses or external runtime dependencies; release builds enable `opt-level = "z"`, `lto = "thin"`, and `strip = true` for a compact single `.exe`. |
| UI & renderer | Slint 1.18.0 (Fluent style, `winit` backend, software renderer) | `.slint` templates compile directly to native code without WebView/Electron browser overhead; CPU software rendering avoids holding GPU shaders/VRAM resident while idling in the background. |
| Local storage | `rusqlite` (embedded SQLite, WAL mode) | Single-file database holds all configuration, collections, and icons without external services; `.cargo/config.toml` trims unused FTS/RTree/JSON1 SQLite modules to keep binary size down. |
| Windows integration | `windows` 0.62 & `tray-icon` | Direct Win32 Shell, DWM, OLE drag-and-drop, global hotkeys, fullscreen detection, and WinHTTP calls enable fine-grained edge animation, window level invariants, and Game DND checks. |
| Minimal features | `image`, `pinyin`, etc. | Only required crate features are enabled (e.g., PNG/JPEG/ICO for `image`, tone-less dictionary for `pinyin`), with heavy extraction work offloaded to worker threads. |

For internals and measured baselines, see [Architecture](docs/en/ARCHITECTURE.md) and [Performance & Boundaries](docs/en/PERFORMANCE.md).

## Core Features

- **Two-level organization & views**: Top-level groups and nested categories supporting programs, files, folders, URLs, `.lnk` shortcuts, and Microsoft Store / UWP apps (`shell:AppsFolder`), with drag-and-drop reordering, cross-group moves, 3 icon grid sizes, and list view.
- **Instant search & full keyboard flow**: Filters over names, aliases, and notes with multi-keyword matching and Chinese Pinyin / initials; supports optional type-to-search, IME composition protection, and end-to-end keyboard navigation from summon to elevated launch.
- **Easy import & batch maintenance**: Drag and drop local files or browser URLs, scan Start Menu and installed Store apps, bulk move/copy/delete items, audit broken targets with in-place relocation, and batch-convert paths between relative and absolute.
- **Edge auto-hide & flexible summon**: Dock to left, right, or top screen edges with configurable 1–12 px visible strip, critically damped slide animation, configurable collapse delay, hide-on-focus-loss, always-on-top, system tray control, global hotkey (`Ctrl+Alt+Space` default), and Windows startup registration.
- **Game Do Not Disturb (DND)**: Automatically detects fullscreen geometry and `SHQueryUserNotificationState` plus up to 100 user-defined strict protected executables to block accidental popups while gaming, with an instant session-only bypass toggle in the tray or Settings.
- **Portable local storage & bilingual UI**: Stores everything in `raptim.db` beside the executable or under `--data-dir`, supports validated JSON export/restore, and speaks both English and Simplified Chinese with automatic OS detection or live manual override.

## Quick Start

### Prerequisites

- Windows 10 / 11 (x64)
- Rust 1.92+ (Edition 2024)
- Visual Studio C++ Build Tools & Windows SDK (including `rc.exe`)

### Build & Run

```powershell
cargo run --release --locked
```

The compiled executable is placed at `target/release/raptim.exe`. Supported launch flags:

```powershell
# Portable mode (same as default: reads and writes in the executable directory)
.\target\release\raptim.exe --portable

# Isolated data directory (ideal for testing, development, or multiple profiles)
.\target\release\raptim.exe --data-dir C:\Temp\RaptimTrial

# Silent startup (collapses to screen edge if docked, otherwise stays hidden in tray)
.\target\release\raptim.exe --silent
```

> **Backup tip**: Raptim does not maintain automatic internal database backups. Before backing up or replacing data, exit Raptim cleanly and copy the entire data directory (including `raptim.db`, `raptim.db-wal`, and `raptim.db-shm`). See [Usage Guide](docs/en/USAGE.md#backups-exportrestore-and-troubleshooting).

## Documentation

| Document | Contents |
| --- | --- |
| [Usage Guide](docs/en/USAGE.md) | CLI flags, daily interactions, shortcuts, all settings, JSON export/restore, and troubleshooting |
| [Architecture](docs/en/ARCHITECTURE.md) | Module layout, threading & events, SQLite schema, `dup_key` deduplication, policy state machine, and Win32/DWM boundaries |
| [Development & Build](docs/en/DEVELOPMENT.md) | Environment setup, Cargo/SQLite build config, headless verification gate, preview generator, tests, and changelog workflow |
| [Performance & Boundaries](docs/en/PERFORMANCE.md) | Storage and startup benchmarks, icon cache budget, edge frame-write findings, and rejected optimizations |
| [Changelog](CHANGELOG.md) | Release index (`zh-CN` & `en-US`; latest [0.1.0](changelog/0.1.0/en-US.md)) |
| [License (GPL-3.0-only)](LICENSE) · [Third-Party Notices](THIRD_PARTY_NOTICES.md) | Full GNU GPLv3 text, direct dependency license review, and asset attribution |

## Verification

Run the headless verification gate to check formatting, Clippy, unit tests, and the release build without popping up any windows:

```powershell
powershell -File scripts/verify.ps1           # fmt + clippy (-D warnings) + debug unit tests + release build
powershell -File scripts/verify.ps1 -Release  # runs clippy and unit tests in release profile
```

See [Development & Build](docs/en/DEVELOPMENT.md#quality-checks-and-verification-gate) for custom `-TargetDir` and `-Jobs` options.

## License

Copyright (C) 2026 KHWLGH

Released under the **GNU General Public License v3.0 (`GPL-3.0-only`)**. See [LICENSE](LICENSE) for full terms. This software is provided as-is without any warranty.

Slint 1.18.0 uses the `GPL-3.0-only` license path (with the official `AboutSlint` component displayed in **Settings → About**), and all remaining dependencies use permissive licenses compatible with `GPL-3.0-only`. The bundled `ui/icons/rust.svg` logo is licensed under CC BY 4.0. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for the complete inventory.

## Links

- [Report Issues or Suggestions](https://github.com/KHWLGH/Raptim/issues)
- [Slint Official Website](https://slint.dev/)
- [Rust Official Website](https://www.rust-lang.org/)
