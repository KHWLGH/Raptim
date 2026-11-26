// Typed event channel between background workers / native hooks and the Slint UI thread.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogKind {
    EditorTarget,
    ProtectedApp,
    ExportJson,
    RestoreJson,
    AddFiles,
    EditorIcon,
    RelocateTarget,
    EditorWorkingDir,
}

pub(super) enum AppEvent {
    Native(platform::NativeSignal, u64),
    WindowInteraction,
    WindowMoved,
    ViewportChanged,
    ThemeChanged,
    EnsureEditorDrop(u64),
    EditorImageDropped {
        generation: u64,
        path: String,
    },
    ExternalDrop(raptim::importing::ExternalDrop),
    StepCategory(i32),
    EditorIconReady {
        epoch: u64,
        requested_target: String,
        icon: Vec<u8>,
    },
    BatchIconsReady {
        epoch: u64,
        icons: Vec<(i64, String, Vec<u8>)>,
    },
    RefreshIconReady {
        epoch: u64,
        id: i64,
        key: String,
        old_icon: Vec<u8>,
        new_icon: Vec<u8>,
    },
    FilesReady(std::result::Result<Vec<Entry>, String>),
    FileSelected {
        kind: DialogKind,
        value: String,
    },
    FilesPicked(Vec<PathBuf>),
    LaunchFinished(Option<Failure>),
    DiscoveredAppsReady(Vec<platform::DiscoveredApp>),
    InstalledAppsReady(Vec<(String, String)>),
    BrokenCheckReady {
        epoch: u64,
        items: Vec<BrokenEntry>,
    },
}

#[derive(Clone)]
pub(super) struct EventSink {
    tx: std::sync::mpsc::Sender<AppEvent>,
    ui: slint::Weak<MainWindow>,
}

impl EventSink {
    pub(super) fn new(tx: std::sync::mpsc::Sender<AppEvent>, ui: slint::Weak<MainWindow>) -> Self {
        Self { tx, ui }
    }

    pub(super) fn send(&self, event: AppEvent) {
        if self.tx.send(event).is_ok() {
            let _ = self.ui.upgrade_in_event_loop(|ui| ui.invoke_flush_events());
        }
    }
}

impl App {
    pub(super) fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Native(signal, ticket) => {
                if signal.requires_ticket() && ticket != self.policy.ticket() {
                    return;
                }
                use platform::NativeSignal;
                match signal {
                    NativeSignal::HotkeyPressed => self.toggle_hotkey_visibility(),
                    NativeSignal::StatusMessage(text) => self.status(text),
                    NativeSignal::TrayShow => self.show_main(ShowSource::Tray),
                    NativeSignal::TraySettings => self.show_settings(),
                    NativeSignal::TrayQuit => self.request_quit(),
                    NativeSignal::TrayRestorePosition => {
                        if self.allowed(ShowSource::Restore) {
                            self.show_main(ShowSource::Restore);
                            self.dock = None;
                            self.edge_hold.clear();
                            self.sync_dock_overhang();
                            platform::restore_position(self.main.window());
                            platform::decorate(self.main.window());
                            windowing::invalidate(self.main.window());
                        }
                    }
                    NativeSignal::SecondaryInstanceSummon => {
                        self.show_main(ShowSource::SecondaryInstance)
                    }
                    NativeSignal::FrameClockTick(generation) => {
                        if self.frame_clock.acknowledge(generation) {
                            self.tick_window(true);
                            self.schedule_interaction();
                        }
                    }
                    NativeSignal::ForegroundChanged | NativeSignal::DisplayChanged => {
                        self.evaluate_suggested()
                    }
                    NativeSignal::TrayTogglePin => {
                        self.main.set_pinned(!self.main.get_pinned());
                        self.apply_pin();
                    }
                    NativeSignal::TrayToggleProtectionBypass => self.toggle_protection_bypass(),
                    NativeSignal::SystemThemeChanged => self.apply_theme(),
                }
            }
            AppEvent::WindowInteraction => {
                self.initialize_native_state();
                self.tick_window(false);
                self.schedule_interaction();
            }
            AppEvent::WindowMoved => {
                self.initialize_native_state();
                // SetWindowPos during animation generates Moved events. Feeding
                // those back into tick_window creates a frame-per-message loop.
                if self.edge_motion.is_none() {
                    self.tick_window(false);
                    self.schedule_interaction();
                }
            }
            AppEvent::ViewportChanged => self.update_visible_icons(),
            AppEvent::ThemeChanged => self.apply_theme(),
            AppEvent::EnsureEditorDrop(generation) => self.install_editor_drop(generation),
            AppEvent::EditorImageDropped { generation, path } => {
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.generation == generation)
                    && !self.file_dialog_open
                    && !self.saving_editor
                    && let Err(e) = self.file_selected(DialogKind::EditorIcon, path)
                    && let Some(editor) = &self.editor
                {
                    editor.window.set_error(e.to_string().into());
                }
            }
            AppEvent::ExternalDrop(drop_data) => self.handle_external_drop(drop_data),
            AppEvent::StepCategory(step) => self.step_category(step),
            AppEvent::EditorIconReady {
                epoch,
                requested_target,
                icon,
            } => {
                if epoch != self.icon_epoch {
                    return;
                }
                self.icon_pending = false;
                if let Some(session) = &mut self.editor {
                    if session.window.get_target().as_str() != requested_target {
                        session.icon_attempted = false;
                        session.window.set_saving(false);
                        session.window.set_error("目标已更改，请重新保存".into());
                        self.saving_editor = false;
                        return;
                    }
                    session.window.set_entry_icon(raptim::icons::display(&icon));
                    session.icon = icon;
                    session.window.set_saving(false);
                }
                if self.saving_editor {
                    self.saving_editor = false;
                    self.save_editor();
                }
            }
            AppEvent::BatchIconsReady { epoch, icons } => {
                if epoch != self.data_epoch {
                    return;
                }
                let mut applied = false;
                for (id, key, icon) in icons {
                    let stale = self
                        .icon_index
                        .get(&id)
                        .and_then(|slot| self.entries.get(slot.index))
                        .is_none_or(|e| !e.icon.is_empty() || e.target.duplicate_key() != key);
                    if stale {
                        continue;
                    }
                    applied |= self.replace_icon(id, icon);
                }
                if applied {
                    self.update_visible_icons();
                }
            }
            AppEvent::RefreshIconReady {
                epoch,
                id,
                key,
                old_icon,
                new_icon,
            } => {
                if epoch != self.data_epoch {
                    return;
                }
                if new_icon.is_empty() {
                    self.status("无法从目标提取图标，保留当前图标");
                    return;
                }
                let usable = self
                    .icon_index
                    .get(&id)
                    .and_then(|slot| self.entries.get(slot.index))
                    .is_some_and(|e| e.target.duplicate_key() == key && e.icon == old_icon);
                if usable && self.replace_icon(id, new_icon) {
                    self.update_visible_icons();
                    self.status("图标缓存已刷新");
                }
            }
            AppEvent::FilesReady(result) => {
                self.importing = false;
                let outcome = result.and_then(|mut entries| {
                    if self.settings.relative_paths {
                        for entry in &mut entries {
                            if let Ok(t) =
                                raptim::paths::convert_target(&entry.target, &self.data_dir, true)
                            {
                                entry.target = t;
                            }
                        }
                    }
                    self.store.add_entries(&entries).map_err(|e| e.to_string())
                });
                match outcome {
                    Ok((added, duplicates)) => {
                        self.status(format!("已添加 {added} 项，跳过 {duplicates} 项重复目标"));
                        self.report_refresh();
                    }
                    Err(e) => self.status(format!("未添加：{e}")),
                }
            }
            AppEvent::FileSelected { kind, value } => {
                if let Err(e) = self.file_selected(kind, value) {
                    self.report_dialog_error(&e);
                }
            }
            AppEvent::FilesPicked(paths) => {
                self.file_dialog_open = false;
                self.focus_hide.reset();
                self.edge_since = Instant::now();
                if !paths.is_empty()
                    && let Err(e) = self.queue_files(paths, self.dialog_collection)
                {
                    self.report_dialog_error(&e);
                }
            }
            AppEvent::LaunchFinished(failure) => {
                self.main.set_busy(false);
                self.evaluate();
                match failure {
                    None => {
                        if self.settings.hide_after_launch && !self.auto_hide_guard().paused() {
                            self.hide_all();
                        }
                    }
                    Some(failure) => {
                        self.status(failure.message.clone());
                        self.show_relocate(failure.entry, &failure.message, failure.relocatable);
                    }
                }
            }
            AppEvent::DiscoveredAppsReady(apps) => {
                self.discovered_checked = vec![false; apps.len()];
                self.discovered_apps = apps;
                self.main.set_discover_loading(false);
                self.render_discover_list();
                if self.main.get_discover_open() {
                    self.main.invoke_focus_discover_filter();
                }
            }
            AppEvent::InstalledAppsReady(apps) => {
                self.installed_apps = apps;
                self.refresh_app_list();
            }
            AppEvent::BrokenCheckReady { epoch, items } => {
                if epoch == self.data_epoch {
                    self.broken_items = items;
                    self.main.set_broken_loading(false);
                    self.sync_broken_ui();
                }
            }
        }
    }

    fn report_dialog_error(&self, e: &Error) {
        self.status(e);
        if let Some(ui) = self.settings_ui.get() {
            ui.set_status(e.to_string().into());
        }
        if let Some(editor) = &self.editor {
            editor.window.set_error(e.to_string().into());
        }
    }
}
