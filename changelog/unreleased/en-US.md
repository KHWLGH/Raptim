[简体中文](zh-CN.md) | **English**

# Unreleased

## Fixed

- Fixed hotkey worker leaving shortcuts registered when disabled, `autostart` registry `HKEY` handle leaks, trailing-backslash Windows argument quoting, and re-enabling autostart after Task Manager disabled `StartupApproved\Run`.
- Fixed `DisplayPolicy` release-debounce recalculation against live thresholds, immediate release on enabling session bypass, and honoring session bypass during elevated launches.
- Fixed `collapsed_probe` pause handling, Fluent context menu re-entrancy and `WM_NCDESTROY` property cleanup, off-by-one non-DWM rounded region bounds and conditional `Raptim.Region` tracking, and hot-unplug monitor enumeration via cached Win32 `EnumDisplayMonitors`; hardened drag-and-drop program/shortcut classification and URL title control-character sanitization.
- Fixed `Rc` reference cycle leak on `on_broken_edit`, broken two-way state sync on `SettingsWindow` slider/select/switch controls, one-way `NavTree` cursor scroll binding, `reveal-result` `visible-height` bounds, `conversion-details` `ScrollView` layout, URL editors auto-fetching favicons on open, `EditorWindow` error tinting (`Theme.danger`), `discover-items` keyboard/accessibility semantics, and focus restoration when closing overlays.
- Fixed scrollbar drag stutter caused by `DragGuard` suppressing initial pointer moves outside `DragArea`, avoided redundant `interaction_timer` teardown and `EventLoopProxy` hops during viewport icon updates (plus faster `Triangle` drag-ghost resizing), and kept docked auto-hide paused while the main window owns Win32 mouse capture with the button held outside window bounds.
- Fixed `normalize_url` scheme detection and ASCII case-insensitive `localhost`, `working_dir` slash normalization in duplicate keys, stale `dup_key` refresh on store open, ID-matched `update_targets_checked`, 256 KiB `save_icon` limit, same-tier/200-char guards in `migrate_delete`, case-insensitive category collision checks, CSV detail escaping/single-header writes, and case-insensitive/raw `DATA_DIR` log redaction.

## Performance & Maintenance

- Converted the unconditional 500 ms `poll_timer` to run on demand, and added manual-reset event gating in `FrameClock`, WinHTTP session/buffer reuse, coalesced `icons::Cache` wakeups with bounded queue short-circuiting, stack-buffered/zero-alloc process matching in `protection`, cached OLE `formats()`, and projection without icon BLOB cloning in broken-item checks.
- Removed no-op Slint software-renderer `drop-shadow-*` declarations that inflated dirty regions, limited item grid instantiation to populated columns per row, moved folder/log/URL shell opens off the UI thread, reused icon ownership via `Store::entries_reusing_icons`, and eliminated redundant per-tick `animations_enabled()`/`request_redraw_now` calls.
- Removed unused `ui/icons/export.svg` and `import.svg`, wired `shortcut.svg` fallback icons for shortcut entries, and trimmed dead UI properties/callbacks and redundant public exports, enum variants, struct fields, and serde derives across library and platform modules.
