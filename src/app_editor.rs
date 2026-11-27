// Project editor window session, icon extraction and form lifecycle.
use super::*;

pub(super) struct EditorSession {
    pub(super) _drop: Option<platform::drop_target::Registration>,
    pub(super) drop_failed: bool,
    pub(super) generation: u64,
    pub(super) window: EditorWindow,
    pub(super) original: Entry,
    pub(super) icon: Vec<u8>,
    pub(super) icon_attempted: bool,
    pub(super) collections: Vec<i64>,
}

impl App {
    pub(super) fn convert_editor_path(&mut self, relative: bool) {
        let Some(entry) = self.editor_draft() else {
            return;
        };
        match raptim::paths::convert_target(&entry.target, &self.data_dir, relative) {
            Ok(target) => {
                if let Some(session) = &self.editor {
                    session.window.set_target(target.value().into());
                    if let Target::Program { working_dir, .. } = target {
                        session.window.set_working_dir(working_dir.into());
                    }
                    session.window.set_error("".into());
                }
            }
            Err(e) => {
                if let Some(session) = &self.editor {
                    session.window.set_error(e.to_string().into());
                }
            }
        }
    }

    pub(super) fn extract_editor_icon(&mut self) {
        let Some(session) = &mut self.editor else {
            return;
        };
        let kind = session.window.get_kind();
        session.icon_attempted = true;
        let requested_target = session.window.get_target().to_string();
        // The lookup only ever needs the target string, so the path kinds share one variant here.
        let value = requested_target.trim().trim_matches('"').to_string();
        let target = match kind {
            3 => Target::Url {
                url: normalize_url(&value).unwrap_or(value),
            },
            5 => Target::App { aumid: value },
            _ => Target::File { path: value },
        };
        self.icon_epoch += 1;
        self.icon_pending = true;
        let epoch = self.icon_epoch;
        let base = self.data_dir.clone();
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let icon = platform::icon_for(&target, &base);
            sink.send(AppEvent::EditorIconReady {
                epoch,
                requested_target,
                icon,
            });
        });
    }

    pub(super) fn editor_draft(&self) -> Option<Entry> {
        let session = self.editor.as_ref()?;
        let ui = &session.window;
        let value = ui.get_target().trim().trim_matches('"').to_string();
        let target = match ui.get_kind() {
            0 => Target::Program {
                path: value,
                arguments: ui.get_arguments().to_string(),
                working_dir: ui.get_working_dir().trim().trim_matches('"').into(),
                elevated: ui.get_elevated(),
                show: ShowMode::from_index(ui.get_show_mode()),
            },
            1 => Target::File { path: value },
            2 => Target::Folder { path: value },
            3 => Target::Url {
                url: normalize_url(&value).unwrap_or(value),
            },
            4 => Target::Shortcut {
                path: value,
                show: ShowMode::from_index(ui.get_show_mode()),
            },
            _ => Target::App { aumid: value },
        };
        Some(Entry {
            icon: session.icon.clone(),
            id: session.original.id,
            position: session.original.position,
            collection_id: *session
                .collections
                .iter()
                .find(|id| **id == i64::from(ui.get_collection_id()))?,
            name: ui.get_entry_name().trim().into(),
            aliases: ui.get_aliases().to_string(),
            notes: ui.get_notes().to_string(),
            target,
        })
    }

    pub(super) fn editor_dirty(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|e| self.editor_draft().as_ref() != Some(&e.original))
    }

    pub(super) fn save_editor(&mut self) {
        if self.icon_pending {
            if let Some(e) = &self.editor {
                e.window.set_error(
                    raptim::i18n::t(
                        "正在读取图标，请稍候保存",
                        "Loading icon; please wait before saving",
                    )
                    .into(),
                );
            }
            return;
        }
        if self.file_dialog_open {
            return;
        }
        let Some(mut entry) = self.editor_draft() else {
            return;
        };
        if entry.id == 0
            && self.settings.relative_paths
            && let Ok(target) = raptim::paths::convert_target(&entry.target, &self.data_dir, true)
        {
            entry.target = target;
        }
        if let Err(e) = entry.validate() {
            if let Some(session) = &self.editor {
                session.window.set_error(e.to_string().into());
            }
            return;
        }
        let fetch_url_icon = matches!(entry.target, Target::Url { .. })
            && self
                .editor
                .as_ref()
                .is_some_and(|session| !session.icon_attempted && session.icon.is_empty());
        if !fetch_url_icon
            && self
                .editor
                .as_ref()
                .is_some_and(|session| !session.icon_attempted && session.icon.is_empty())
        {
            self.saving_editor = true;
            if let Some(session) = &self.editor {
                session.window.set_saving(true);
            }
            self.extract_editor_icon();
            return;
        }
        match self.store.save_entry(&entry) {
            Ok(saved_id) => {
                let target = entry.target.clone();
                self.drop_editor();
                self.report_refresh();
                if fetch_url_icon {
                    self.fill_icons(vec![(saved_id, target)]);
                }
                if self.quitting {
                    self.request_quit();
                }
            }
            Err(e) => {
                if let Some(session) = &self.editor {
                    session.window.set_error(e.to_string().into());
                }
            }
        }
    }

    pub(super) fn cancel_editor(&mut self) {
        if self.saving_editor {
            self.saving_editor = false;
            self.icon_epoch += 1;
            self.icon_pending = false;
            if let Some(session) = &self.editor {
                session.window.set_saving(false);
            }
        }
        if self.file_dialog_open {
            return;
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|session| session.window.get_discard_confirm())
        {
            if let Some(session) = &self.editor {
                session.window.set_discard_confirm(false);
                session.window.invoke_focus_name();
            }
            self.quitting = false;
            return;
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|session| session.window.get_collection_open())
        {
            if let Some(session) = &self.editor {
                session.window.set_collection_open(false);
                session.window.invoke_focus_name();
            }
            return;
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|session| session.window.get_picker_open())
        {
            self.close_app_picker();
            return;
        }
        if self.editor_dirty() {
            self.editor
                .as_ref()
                .unwrap()
                .window
                .set_discard_confirm(true);
        } else {
            self.drop_editor();
            if self.quitting {
                self.request_quit();
            }
        }
    }

    pub(super) fn drop_editor(&mut self) {
        self.focus_hide.reset();
        self.edge_since = Instant::now();
        self.icon_epoch += 1;
        self.icon_pending = false;
        self.saving_editor = false;
        self.editor_generation = self.editor_generation.wrapping_add(1);
        if let Some(mut session) = self.editor.take() {
            session._drop.take();
            let _ = windowing::close_child(session.window.window(), self.main.window());
        }
    }
}

pub(super) fn open_editor(app: &Rc<RefCell<App>>, id: i64) {
    let mut state = app.borrow_mut();
    if id != 0 && state.main.get_batch_mode() {
        return;
    }
    if state.file_dialog_open {
        state.status(raptim::i18n::t(
            "请先关闭文件选择窗口",
            "Please close the file dialog first",
        ));
        return;
    }
    if !state.allowed(ShowSource::Editor) {
        return;
    }
    state.user_summon_edge();
    if let Some(session) = &state.editor
        && (session.original.id == id || state.editor_dirty())
    {
        if session.original.id != id {
            session.window.set_error(
                raptim::i18n::t(
                    "请先保存或取消当前编辑",
                    "Save or discard the current edit first",
                )
                .into(),
            );
        }
        session.window.set_topmost_window(state.child_topmost());
        let _ = state.show_child_window(session.window.window());
        state.apply_theme();
        return;
    }
    state.drop_editor();
    let entry = if id == 0 {
        Entry {
            icon: Vec::new(),
            id: 0,
            collection_id: state.collection,
            name: String::new(),
            aliases: String::new(),
            notes: String::new(),
            target: Target::Program {
                path: String::new(),
                arguments: String::new(),
                working_dir: String::new(),
                elevated: false,
                show: ShowMode::default(),
            },
            position: 0,
        }
    } else {
        let Some(e) = state.entries.iter().find(|e| e.id == id) else {
            return;
        };
        e.clone()
    };
    let ui = match EditorWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            state.status(e);
            return;
        }
    };
    ui.global::<I18n>().set_en(raptim::i18n::is_en());
    ui.set_entry_icon(raptim::icons::display(&entry.icon));
    ui.set_entry_name(entry.name.clone().into());
    ui.set_target(entry.target.value().into());
    ui.set_aliases(entry.aliases.clone().into());
    ui.set_notes(entry.notes.clone().into());
    ui.set_editing(id != 0);
    ui.set_kind(entry.target.kind_index());
    ui.set_show_mode(match &entry.target {
        Target::Program { show, .. } | Target::Shortcut { show, .. } => show.index(),
        _ => 0,
    });
    if let Target::Program {
        arguments,
        working_dir,
        elevated,
        ..
    } = &entry.target
    {
        ui.set_arguments(arguments.clone().into());
        ui.set_working_dir(working_dir.clone().into());
        ui.set_elevated(*elevated);
    }
    let collections: Vec<_> = state
        .collections
        .iter()
        .filter(|c| c.parent_id.is_some())
        .map(|c| c.id)
        .collect();
    ui.set_collection_id(entry.collection_id as i32);
    ui.set_collection_name(state.collection_name(entry.collection_id));
    ui.set_collection_open(false);
    ui.set_collection_cursor(0);
    macro_rules! bind {
        ($method:ident, $action:expr) => {{
            let weak = Rc::downgrade(app);
            ui.$method(move || {
                if let Some(app) = weak.upgrade() {
                    ($action)(&mut app.borrow_mut());
                }
            });
        }};
    }
    bind!(on_pick_app, |s: &mut App| s.open_app_picker());
    bind!(on_pick_collection, |s: &mut App| s.pick_collection());
    bind!(on_picker_refilter, |s: &mut App| s.refresh_app_list());
    {
        let weak = Rc::downgrade(app);
        ui.on_picker_confirm(move |aumid, name| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut()
                    .choose_app(aumid.to_string(), name.to_string());
            }
        });
    }
    {
        let weak = Rc::downgrade(app);
        ui.on_collection_node(move |id| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().collection_node(id);
            }
        });
        let weak = Rc::downgrade(app);
        ui.on_collection_key(move |delta| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().collection_key(delta);
            }
        });
    }
    bind!(on_save, |s: &mut App| s.save_editor());
    bind!(on_cancel, |s: &mut App| s.cancel_editor());
    bind!(on_browse, |s: &mut App| s.dialog(DialogKind::EditorTarget));
    bind!(on_browse_working_dir, |s: &mut App| s
        .dialog(DialogKind::EditorWorkingDir));
    bind!(on_browse_icon, |s: &mut App| s
        .dialog(DialogKind::EditorIcon));
    bind!(on_reset_icon, |s: &mut App| s.extract_editor_icon());
    let weak = Rc::downgrade(app);
    ui.on_convert_path(move |relative| {
        if let Some(app) = weak.upgrade() {
            app.borrow_mut().convert_editor_path(relative);
        }
    });
    bind!(on_discard, |s: &mut App| {
        s.drop_editor();
        if s.quitting {
            s.request_quit();
        }
    });
    bind!(on_keep_editing, |s: &mut App| {
        s.quitting = false;
    });
    let weak = Rc::downgrade(app);
    ui.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade() {
            app.borrow_mut().cancel_editor();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let drop_sink = state.events.clone();
    let generation = state.editor_generation;
    let weak = ui.as_weak();
    let compose_weak = ui.as_weak();
    install_keys(
        ui.window(),
        state.events.clone(),
        move |key| {
            use slint::winit_030::winit::keyboard::{Key, NamedKey};
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            if key.logical == &Key::Named(NamedKey::Escape) {
                ui.invoke_cancel();
                true
            } else if key.modifiers.control_key()
                && !key.modifiers.alt_key()
                && key.logical == &Key::Named(NamedKey::Enter)
                && !ui.get_discard_confirm()
                && !ui.get_collection_open()
                && !ui.get_picker_open()
            {
                ui.invoke_save();
                true
            } else {
                false
            }
        },
        move |event| {
            if matches!(
                event,
                slint::winit_030::winit::event::WindowEvent::Focused(_)
                    | slint::winit_030::winit::event::WindowEvent::Resized(_)
                    | slint::winit_030::winit::event::WindowEvent::RedrawRequested
            ) {
                drop_sink.send(AppEvent::EnsureEditorDrop(generation));
            }
        },
        move |active| {
            if let Some(ui) = compose_weak.upgrade() {
                ui.set_composing(active);
            }
        },
        None,
    );
    ui.set_topmost_window(state.child_topmost());
    ui.window().set_size(slint::LogicalSize::new(520., 650.));
    state.editor = Some(EditorSession {
        _drop: None,
        drop_failed: false,
        generation,
        window: ui.clone_strong(),
        icon: entry.icon.clone(),
        icon_attempted: false,
        original: entry,
        collections,
    });
    if let Err(e) = state.show_child_window(ui.window()) {
        state.status(e);
    } else {
        state.apply_theme();
    }
    state.sync_poll_timer();
    let sink = state.events.clone();
    slint::Timer::single_shot(Duration::ZERO, move || {
        sink.send(AppEvent::EnsureEditorDrop(generation));
    });
    if state.editor.as_ref().is_some_and(|e| {
        e.window.get_kind() != 3 && e.icon.is_empty() && !e.window.get_target().is_empty()
    }) {
        state.extract_editor_icon();
    }
}
