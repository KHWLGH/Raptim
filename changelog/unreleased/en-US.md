[简体中文](zh-CN.md) | **English**

# Unreleased

### Added

- Added full English UI support alongside Simplified Chinese (`src/i18n.rs` + Slint `I18n` global): automatically detects Windows UI language (`PreferredUILanguages` / `UserProfile\Languages` / `LocaleName`) on first launch, adapts starter group/category names (`Common / Default` or `常用 / 默认分类`), switches UI fonts between `Segoe UI` and `Microsoft YaHei UI`, and supports live switching (`Follow system` / `简体中文` / `English`) across all windows, context menus, tray menu, and dialogs under **Settings → General → Language**.
- Added complete English documentation (`README.en.md`, `docs/en/{USAGE,ARCHITECTURE,DEVELOPMENT,PERFORMANCE}.md`, `changelog/{0.1.0,unreleased}/en-US.md`) and bilingual switchers across all Chinese docs and `CHANGELOG.md`.
- Added `--lang zh|en` (`-Lang` in `scripts/preview.ps1`) to the completely headless, non-foreground-stealing UI preview generator (`examples/ui_preview/`) and generated English preview images under `docs/previews/en/`.
- Added headless UI preview generator (`examples/ui_preview/` and `scripts/preview.ps1`) using Slint's software renderer offscreen without creating OS windows or stealing foreground focus, producing Light/Dark PNG screenshots, overview grids, and a self-contained `index.html` gallery (`showcase`/`all` presets, `window`/`card`/`raw` frames, and `--data-dir` / `--import-json` real-data support).
