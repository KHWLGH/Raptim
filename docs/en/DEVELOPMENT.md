← Back to [README](../../README.en.md) · [简体中文](../DEVELOPMENT.md) | **English**

# Development & Build Guide

This guide covers Raptim's development environment, build configuration, headless verification scripts, preview generator, test boundaries, and changelog workflow.

- [Prerequisites](#prerequisites)
- [Building & Running](#building--running)
- [Quality Checks and Verification Gate](#quality-checks-and-verification-gate)
- [Headless UI Preview Generator](#headless-ui-preview-generator)
- [Testing & Regression Boundaries](#testing--regression-boundaries)
- [Commits & Delivery Checklist](#commits--delivery-checklist)
- [Versioning & Changelog Workflow](#versioning--changelog-workflow)
- [Third-Party Dependencies & License Compliance](#third-party-dependencies--license-compliance)

## Prerequisites

| Component | Requirement | Notes |
| --- | --- | --- |
| Operating system | Windows 10 / 11 (x64) | Windows-only target; application manifest enables PerMonitorV2 DPI |
| Rust toolchain | Rust 1.92+ (Edition 2024) | `Cargo.toml` sets `rust-version = "1.92"` (verified on Rust 1.98.1) |
| C++ toolchain | Visual Studio C++ Build Tools + Windows SDK | Needed for MSVC linking and compiling `raptim.rc` / `libsqlite3-sys` (`rc.exe` required) |
| Lockfile | `Cargo.lock` (use `--locked`) | `slint`, `i-slint-core`, and `slint-build` are pinned strictly to `=1.18.0` |

## Building & Running

```powershell
git clone https://github.com/KHWLGH/Raptim.git
cd Raptim

# Run release build using the default executable directory for data
cargo run --release --locked

# Run with an isolated data directory so daily config stays untouched
cargo run --release --locked -- --data-dir C:\Temp\RaptimTrial
```

Build the release executable only:

```powershell
cargo build --release --locked
```

Output binary: `target/release/raptim.exe` (release profile sets `opt-level = "z"`, `lto = "thin"`, and `strip = true`).

### Build Configuration Notes

- **`.cargo/config.toml` (do not remove)**: Passes `/USQLITE_ENABLE_FTS5 /USQLITE_ENABLE_FTS3 ... /DSQLITE_OMIT_JSON /DSQLITE_OMIT_LOAD_EXTENSION` via `[env] CFLAGS` to omit unused SQLite modules (FTS, RTree, JSON1, dynamic extensions) and keep binary size compact.
- **`build.rs`**: Locates Windows SDK `rc.exe` to compile `raptim.rc` (embedding `ui/icons/raptim.ico`), embeds `raptim.manifest` via linker arguments, records `rustc --version` into `RAPTIM_RUST_VERSION` for the About page, and compiles `ui/app.slint` using the `fluent` style.

## Quality Checks and Verification Gate

`scripts/verify.ps1` provides a fully headless (zero window popups) verification gate kept strictly ASCII-only for Windows PowerShell 5.1 compatibility:

```powershell
# Default gate: rustfmt + clippy (-D warnings) + debug unit tests + release build + exe size
powershell -File scripts/verify.ps1

# Same pipeline with clippy and unit tests built under release profile
powershell -File scripts/verify.ps1 -Release
```

> If your Windows PowerShell execution policy blocks unsigned local scripts, invoke `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1`.

Checks executed in order (aborts immediately on first failure):

1. `cargo fmt --check`
2. `cargo clippy --all-targets --locked -- -D warnings`
3. `cargo test --all-targets --locked`
4. `cargo build --release --locked` (prints `raptim.exe` size and timestamp)

### Targeted Unit Tests

For fast iteration when editing a single module (still run `scripts/verify.ps1` before finishing):

```powershell
cargo test --lib --locked
cargo test --lib --locked storage::
cargo test --lib --locked menus::
cargo test --lib --locked i18n::
cargo test --bin raptim --locked
```

### Handling Windows Defender / Antivirus Locks

If compilation hits `os error 32` or `Access is denied` (commonly triggered by real-time scanning of `build-script-build.exe`), `scripts/verify.ps1` catches and reports it. Either:

- Exclude your repo `target`, `~/.cargo`, and `~/.rustup` folders from real-time scanning, or
- Use a separate target directory and single job concurrency:

```powershell
powershell -File scripts/verify.ps1 -TargetDir C:\Temp\raptim-target -Jobs 1
```

## Headless UI Preview Generator

`examples/ui_preview/` (wrapped by `scripts/preview.ps1`) renders screenshots directly into memory pixel buffers via Slint's software renderer without creating any Win32 HWND or stealing foreground focus:

```powershell
# Render showcase scenes (zh default, Light + Dark, 2x DPI, Windows corners + shadows) + HTML gallery
powershell -File scripts/preview.ps1

# Render all 21 scenes in English across both themes
powershell -File scripts/preview.ps1 -Preset all -Lang en -Theme both

# Use backdrop card framing or select specific scenes
powershell -File scripts/preview.ps1 -Lang en -Frame card -Scenes "main-grid,main-search,settings-about"

# Render real items from an existing Raptim data dir or JSON export (never creates an empty DB)
powershell -File scripts/preview.ps1 -Lang en -DataDir C:\Temp\RaptimTrial
powershell -File scripts/preview.ps1 -Lang en -ImportJson C:\Temp\raptim-export.json
```

Inspect all flags and scene IDs directly via Cargo:

```powershell
cargo run --release --locked --example ui_preview -- --list --lang en
cargo run --release --locked --example ui_preview -- --help
```

- Default output directory is `target/ui-previews` (git-ignored) containing `light/<scene>.png`, `dark/<scene>.png`, `overview-light.png` / `overview-dark.png`, `manifest.json`, and a self-contained `index.html` gallery (theme split/light/dark switcher, group tabs, lightbox preview, path/Markdown copy buttons).
- `scripts/preview.ps1` stays strictly pure ASCII and never opens a browser automatically unless `-Open` is explicitly passed.

## Testing & Regression Boundaries

Automated tests focus on **core pure logic and data invariants**, avoiding fragile UI pixel, coordinate, or static copy snapshots:

- **Covered by automated unit tests**:
  - `model` / `paths` / `storage`: target validation, URL completion, relative/absolute path roundtrips, unified `dup_key` checking, opening legacy duplicate DBs safely, transactional JSON export/restore, and settings validation.
  - `search`: target-path exclusion, Pinyin/initials/camelCase/multi-word ranking, incremental index sync.
  - `i18n` / `menus`: BCP-47 locale tag parsing, lock-free locale resolution (tests never mutate global `ACTIVE_LOCALE`), menu command uniqueness & bidirectional `region()` routing across Chinese and English trees.
  - `docking` / `windowing` / `policy` / `session`: edge overhang geometry, critically damped motion, focus-loss fade, physical-to-logical dirty rect conversion, protection rule precedence, polling intervals, and `NoticeQueue` coalescing.
  - `gestures` / `diagnostics` / `platform` / `main`: 180 ms / 12 DIP drag guard, IME composition guard, log redaction and half-log line-boundary truncation, `.lnk`/`.url` roundtrip, `<link>` favicon tag parsing and HTTP status check, launch failure classification, and printable character filtering.
- **Requires manual testing on real hardware**:
  - Browser OLE drag-and-drop (Edge / Chrome / Firefox) and URL clipboard flavors.
  - Multi-monitor hotplug, mixed-DPI dragging, and DWM compositor visual behavior during edge slides.
  - Windows Narrator and High Contrast themes.
  - Fullscreen Game DND trigger and release across exclusive fullscreen / borderless games (such as `osu!lazer`).

## Commits & Delivery Checklist

- **Commit prefixes**: `feat:`, `fix:`, `docs:`, `perf:`, `refactor:`, `test:`, `chore:`.
- **Before finishing**:
  - Review `git status` and `git diff` to ensure no stray test databases or temporary files sit outside `target/` (`.gitignore` only ignores `/target`).
  - Whenever behavior, settings, shortcuts, storage schema, or build/verify steps change, update the matching `docs/` & `docs/en/` pages and `changelog/unreleased/{zh-CN,en-US}.md`.
  - Clearly distinguish automated checks actually executed from scenarios requiring real-device manual verification.

## Versioning & Changelog Workflow

Raptim uses a single root index (`CHANGELOG.md`) pointing to per-version and unreleased files under `changelog/<version>/{zh-CN,en-US}.md`:

- **Everyday development**: Update [changelog/unreleased/zh-CN.md](../../changelog/unreleased/zh-CN.md) and [changelog/unreleased/en-US.md](../../changelog/unreleased/en-US.md) (`Added` / `Changed` / `Fixed` / `Performance & Maintenance`). Relative links inside those files resolve from their own folder (for example, `../../docs/en/USAGE.md`).
- **Preparing a release**:
  1. Finalize `changelog/unreleased/{zh-CN,en-US}.md` and archive them to `changelog/<version>/{zh-CN,en-US}.md` (top heading `# <version> - YYYY-MM-DD`).
  2. Insert the new version under `<!-- releases:start -->` in [CHANGELOG.md](../../CHANGELOG.md), keep `[Unreleased]`, and reset `changelog/unreleased/{zh-CN,en-US}.md`.
  3. Update `version` in [Cargo.toml](../../Cargo.toml) and [Cargo.lock](../../Cargo.lock) (`Settings → About` reads `CARGO_PKG_VERSION` automatically).
  4. Run `powershell -File scripts/verify.ps1 -Release` and verify all documentation links before committing or tagging.

> Routine feature/docs work does not bump versions or archive releases.

## Third-Party Dependencies & License Compliance

- Raptim's own code and combined binaries are licensed under **GNU General Public License v3.0 only (`GPL-3.0-only`)**. Slint 1.18.0 uses the `GPL-3.0-only` path (retaining `AboutSlint` in **Settings → About**), and the full license lives in [LICENSE](../../LICENSE).
- When adding or updating Cargo dependencies, verify transitive license compatibility offline via `cargo metadata --locked --offline --filter-platform x86_64-pc-windows-msvc` and update [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md).
- Binary distributions must comply with GPLv3 Section 6 by shipping [LICENSE](../../LICENSE), [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md), and exact corresponding source code (including `Cargo.lock`).
