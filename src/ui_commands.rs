// UI command adapters; shared storage operations live in the library.
use super::*;
use raptim::menus::{self, MenuContext, MenuDestination, MenuEntry, MenuNode, Region};

impl App {
    pub(super) fn category_for(&self, id: i64) -> i64 {
        self.collections
            .iter()
            .find(|c| c.id == id)
            .filter(|c| c.parent_id.is_some())
            .map(|c| c.id)
            .or_else(|| {
                self.collections
                    .iter()
                    .find(|c| c.parent_id == Some(id))
                    .map(|c| c.id)
            })
            .unwrap_or(self.collection)
    }
    pub(super) fn cancel_inline(&mut self) {
        self.main.set_inline_kind(0);
        self.main.set_inline_error("".into());
        // The name field lives in this window now, so focus has to be moved back explicitly.
        self.main.invoke_focus_browser();
        self.focus_hide.reset();
        self.edge_since = Instant::now();
    }
    pub(super) fn finish_inline(&mut self) -> bool {
        let kind = self.main.get_inline_kind();
        if kind == 0 {
            return true;
        }
        let id = i64::from(self.main.get_inline_id());
        let name = self.main.get_inline_text().to_string();
        let result = match kind {
            1 => self
                .collections
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| {
                    Error::Invalid(
                        raptim::i18n::t("分类已不存在", "Category no longer exists").into(),
                    )
                })
                .and_then(|c| self.store.save_collection(Some(id), c.parent_id, &name)),
            3 => self.store.save_collection(None, None, &name),
            _ => self.store.save_collection(None, Some(id), &name),
        };
        match result {
            Ok(created) => {
                if kind == 3 {
                    self.group = created;
                    self.collection = created;
                }
                if kind == 4 {
                    self.group = id;
                    self.collection = created;
                }
                self.cancel_inline();
                self.report_refresh();
                true
            }
            Err(e) => {
                self.main.set_inline_error(e.to_string().into());
                false
            }
        }
    }
    pub(super) fn current_scope(&self) -> std::collections::HashSet<i64> {
        std::iter::once(self.collection).collect()
    }

    /// Snapshot what a context menu needs. `groups` and `destinations` are derived with the same
    /// iterators that publish the Slint models in `rebuild_navigation`, because a menu command
    /// number is an index into those lists.
    pub(super) fn menu_context(&self, kind: i32, id: i32) -> MenuContext {
        let holds = |collection: i64| {
            self.entries
                .iter()
                .any(|e| e.id == i64::from(id) && e.collection_id == collection)
        };
        let destinations = self
            .collections
            .iter()
            .filter_map(|c| Some((c, c.parent_id?)))
            .map(|(c, parent)| MenuDestination {
                id: c.id as i32,
                name: c.name.clone(),
                parent: parent as i32,
                holds_entry: holds(c.id),
            })
            .collect();
        let siblings = |ids: Vec<i64>| movement(&ids, i64::from(id));
        let (target, can_move_up, can_move_down) = match kind {
            1 => match self.entries.iter().find(|e| e.id == i64::from(id)) {
                Some(entry) => {
                    let (up, down) = siblings(
                        self.entries
                            .iter()
                            .filter(|s| s.collection_id == entry.collection_id)
                            .map(|s| s.id)
                            .collect(),
                    );
                    (
                        Some(MenuEntry {
                            id,
                            kind: entry.target.kind_index(),
                        }),
                        up,
                        down,
                    )
                }
                None => (None, false, false),
            },
            2 | 3 => match self.collections.iter().find(|c| c.id == i64::from(id)) {
                Some(collection) => {
                    let (up, down) = siblings(
                        self.collections
                            .iter()
                            .filter(|c| c.parent_id == collection.parent_id)
                            .map(|c| c.id)
                            .collect(),
                    );
                    (None, up, down)
                }
                None => (None, false, false),
            },
            _ => (None, false, false),
        };
        MenuContext {
            target,
            list_view: self.main.get_list_view(),
            icon_size: self.main.get_icon_size(),
            batch_blocked: self.editor.is_some() || self.importing || self.file_dialog_open,
            can_move_up,
            can_move_down,
            groups: self
                .collections
                .iter()
                .filter(|c| c.parent_id.is_none())
                .map(|c| MenuNode {
                    id: c.id as i32,
                    name: c.name.clone(),
                })
                .collect(),
            destinations,
        }
    }

    pub(super) fn begin_collection_action(&mut self, id: i32, action: i32, target: i32) {
        // Checked first: `finish_inline` below can write to the store, and the name form this
        // ends up showing must not appear over a game.
        if !self.allowed(ShowSource::Editor) {
            return;
        }
        if self.editor.is_some() {
            self.status(raptim::i18n::t(
                "请先完成项目编辑",
                "Finish editing the item first",
            ));
            return;
        }
        if !self.finish_inline() {
            return;
        }
        self.main.set_inline_error("".into());
        let source = self
            .collections
            .iter()
            .find(|c| c.id == i64::from(id))
            .cloned();
        match action {
            0 | 1 => {
                self.main.set_inline_kind(if action == 0 { 3 } else { 4 });
                self.main.set_inline_id(
                    source
                        .as_ref()
                        .map_or(self.group, |c| c.parent_id.unwrap_or(c.id))
                        as i32,
                );
                self.main
                    .set_inline_context(self.collection_name(i64::from(self.main.get_inline_id())));
                self.main.set_inline_text("".into());
            }
            2 => {
                if let Some(c) = source {
                    self.main.set_inline_kind(1);
                    self.main.set_inline_id(id);
                    self.main.set_inline_text(c.name.into());
                }
            }
            3 => {
                if let Some(c) = source {
                    self.delete_collection_id = c.id;
                    self.main.set_delete_name(c.name.clone().into());
                    self.main.set_delete_is_group(c.parent_id.is_none());
                    self.main.set_delete_has_content(
                        self.entries.iter().any(|e| e.collection_id == c.id)
                            || self.collections.iter().any(|d| d.parent_id == Some(c.id)),
                    );
                    let destinations: Vec<_> = self
                        .collections
                        .iter()
                        .filter(|d| {
                            d.id != c.id
                                && d.parent_id != Some(c.id)
                                && (if c.parent_id.is_some() {
                                    d.parent_id.is_some()
                                } else {
                                    d.parent_id.is_none()
                                })
                        })
                        .map(|d| NavItem {
                            id: d.id as i32,
                            name: self.collection_name(d.id),
                        })
                        .collect();
                    self.main
                        .set_delete_names(strings(destinations.iter().map(|d| d.name.to_string())));
                    self.main
                        .set_delete_destinations(Rc::new(VecModel::from(destinations)).into());
                    self.main.set_delete_target(0);
                    self.main.set_delete_discard(false);
                    self.main.set_collection_delete(true);
                }
            }
            4 => {
                if let Some(c) = source
                    && c.parent_id.is_some()
                {
                    if let Err(e) =
                        self.store
                            .save_collection(Some(c.id), Some(i64::from(target)), &c.name)
                    {
                        self.status(e);
                    }
                    self.report_refresh();
                }
            }
            5 => {
                if let Some(c) = source {
                    let siblings: Vec<_> = self
                        .collections
                        .iter()
                        .filter(|d| d.parent_id == c.parent_id)
                        .map(|d| d.id)
                        .collect();
                    if let Some(i) = siblings.iter().position(|d| *d == c.id) {
                        let result = if target < 0 && i > 0 {
                            self.store.reorder_collection(c.id, siblings[i - 1])
                        } else if target > 0 && i + 1 < siblings.len() {
                            self.store.reorder_collection(siblings[i + 1], c.id)
                        } else {
                            Ok(())
                        };
                        if let Err(e) = result {
                            self.status(e);
                        }
                        self.report_refresh();
                    }
                }
            }
            _ => {}
        }
        if self.main.get_inline_kind() != 0 {
            self.user_summon_edge();
            self.main.set_inline_error("".into());
            self.main.invoke_focus_inline_name();
        }
    }
    pub(super) fn update_conversion(&mut self) {
        let mut nodes = Vec::new();
        for group in self.collections.iter().filter(|c| c.parent_id.is_none()) {
            let children: Vec<_> = self
                .collections
                .iter()
                .filter(|c| c.parent_id == Some(group.id))
                .collect();
            let ids: Vec<_> = children.iter().map(|c| c.id).collect();
            let selected = ids
                .iter()
                .filter(|id| self.scope_selection.contains(id))
                .count();
            nodes.push(ScopeNode {
                id: -(group.id as i32),
                parent: 0,
                label: group.name.clone().into(),
                depth: 0,
                state: if selected == 0 {
                    0
                } else if selected == ids.len() {
                    2
                } else {
                    1
                },
            });
            nodes.extend(children.iter().map(|c| ScopeNode {
                id: c.id as i32,
                parent: group.id as i32,
                label: c.name.clone().into(),
                depth: 1,
                state: if self.scope_selection.contains(&c.id) {
                    2
                } else {
                    0
                },
            }));
        }
        self.main.set_scopes(Rc::new(VecModel::from(nodes)).into());
        let preview = raptim::commands::conversion_preview(
            &self.entries,
            &self.scope_selection,
            &self.data_dir,
            self.main.get_conversion_relative(),
        );
        let summary = if raptim::i18n::is_en() {
            format!(
                "Selected {} scope(s) · {} item(s)\nConvertible {} · Unchanged {} · Skipped {}",
                self.scope_selection.len(),
                preview.selected.len(),
                preview.entries.len(),
                preview.unchanged,
                preview.skipped.len()
            )
        } else {
            format!(
                "已选 {} 个位置 · {} 个项目\n可转换 {} · 无需修改 {} · 跳过 {}",
                self.scope_selection.len(),
                preview.selected.len(),
                preview.entries.len(),
                preview.unchanged,
                preview.skipped.len()
            )
        };
        self.main.set_conversion_summary(summary.into());
        self.main
            .set_conversion_details(preview.skipped.join("\n").into());
        self.main
            .set_scope_selected(!self.scope_selection.is_empty() && !preview.entries.is_empty());
        self.conversion_preview = Some(preview);
    }
}

/// `Store::move_entry` and the collection reorder both stop silently at either end, so the menu
/// refuses before the user pays for a no-op. `ids` must be the same ordered sibling list the
/// reorder reads: same-collection entries by position for a project, same-parent collections for a
/// group or category.
fn movement(ids: &[i64], id: i64) -> (bool, bool) {
    match ids.iter().position(|other| *other == id) {
        Some(index) => (index > 0, index + 1 < ids.len()),
        None => (false, false),
    }
}

/// Routes the list-management block the two grid menus share.
fn dispatch_manage(ui: &MainWindow, command: u32) {
    match command {
        30 => ui.invoke_batch_start(0),
        40 => ui.invoke_add(),
        41 => ui.invoke_add_files(),
        42..=45 => ui.invoke_view_change(command as i32 - 42),
        46 => ui.invoke_global_action(7),
        47 => ui.invoke_discover_apps(),
        _ => {}
    }
}

/// Application-wide commands, offered only by the title-bar 更多 menu.
fn dispatch_app(ui: &MainWindow, command: u32) {
    match command {
        1 => ui.invoke_global_action(2),
        2 => ui.invoke_global_action(4),
        3 => ui.invoke_global_action(8),
        4 => ui.invoke_global_action(5),
        5 => ui.invoke_global_action(6),
        6 | 7 => ui.invoke_begin_conversion(command == 6),
        8 => ui.invoke_settings(),
        9 => ui.invoke_quit(),
        _ => {}
    }
}

fn dispatch_item(
    ui: &MainWindow,
    ctx: &MenuContext,
    id: i32,
    name: slint::SharedString,
    command: u32,
) {
    let target = |base: u32| {
        ctx.destinations
            .get((command - base) as usize)
            .map(|d| d.id)
    };
    match command {
        1 => ui.invoke_launch(id),
        2 => ui.invoke_item_action(id, 0),
        3 => ui.invoke_item_action(id, 1),
        4 => ui.invoke_edit(id),
        11 => ui.invoke_item_action(id, 7),
        12..=15 => ui.invoke_item_action(id, command as i32 - 9),
        6 | 7 => ui.invoke_convert_path(id, command == 6),
        8 | 9 => ui.invoke_move(id, if command == 8 { -1 } else { 1 }),
        10 => {
            ui.set_selected_id(id);
            ui.set_delete_name(name);
            ui.set_inline_error("".into());
            ui.set_confirm_delete(true);
        }
        // 复制到 is tested first because its command numbers satisfy `>= MOVE_BASE` too, and
        // 移动到 can only be that when the destination list is longer than the gap between bases.
        _ if command >= menus::COPY_BASE => {
            if let Some(target) = target(menus::COPY_BASE) {
                ui.invoke_copy_to(id, target);
            }
        }
        _ if command >= menus::MOVE_BASE => {
            if let Some(target) = target(menus::MOVE_BASE) {
                ui.invoke_move_to(id, target);
            }
        }
        _ => {}
    }
}

fn dispatch_collection(ui: &MainWindow, ctx: &MenuContext, id: i32, kind: i32, command: u32) {
    match command {
        1 => ui.invoke_collection_action(id, if kind == 2 { 0 } else { 1 }, 0),
        2 => ui.invoke_collection_action(id, 1, 0),
        3 => ui.invoke_collection_action(id, 2, 0),
        4 | 5 => ui.invoke_collection_action(id, 5, if command == 4 { -1 } else { 1 }),
        6 => ui.invoke_collection_action(id, 3, 0),
        n if n >= menus::MOVE_BASE => {
            if let Some(group) = ctx.groups.get((n - menus::MOVE_BASE) as usize) {
                ui.invoke_collection_action(id, 4, group.id);
            }
        }
        _ => {}
    }
}

pub(super) fn bind_ui_commands(app: &Rc<RefCell<App>>) {
    let main = app.borrow().main.clone_strong();
    let weak = Rc::downgrade(app);
    main.on_native_menu(move |kind, id| {
        let weak = weak.clone();
        // Finish the pointer-release dispatch before entering a native modal loop.
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            // The pointer release and this hop are not atomic with respect to protection.
            if !app.borrow_mut().allowed(ShowSource::NativeMenu) {
                return;
            }
            let ui = app.borrow().main.clone_strong();
            if kind == 1 && ui.get_batch_mode() {
                return;
            }
            // Everything a tree needs is snapshotted in one borrow and released, so no `RefCell`
            // handle is held while the native modal loop runs; the router then resolves a command
            // against the same numbering the tree was built from.
            let ctx = app.borrow().menu_context(kind, id);
            let name = ui
                .get_items()
                .iter()
                .chain(ui.get_results().iter())
                .find(|e| e.id == id)
                .map_or_else(Default::default, |e| e.name);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerExited);
            let items = menus::tree(kind, &ctx);
            ui.set_more_open(true);
            let selected = platform::show_menu(
                ui.window(),
                &items,
                ui.global::<Palette>().get_background().color(),
            );
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerExited);
            ui.set_more_open(false);
            let Some(command) = selected else {
                return;
            };
            match menus::region(kind, command) {
                Some(Region::Manage) => dispatch_manage(&ui, command),
                Some(Region::App) => dispatch_app(&ui, command),
                Some(Region::Item) => dispatch_item(&ui, &ctx, id, name, command),
                Some(Region::Collection) => dispatch_collection(&ui, &ctx, id, kind, command),
                // A row whose number no region claims would otherwise swallow the click silently.
                None => raptim::diagnostics::error(
                    "menu-unrouted",
                    &format!("kind {kind} command {command}"),
                ),
            }
        });
    });
    macro_rules! bind { ($callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => { { let weak = Rc::downgrade(app); main.$callback(move |$($arg),*| { if let Some(app) = weak.upgrade() { #[allow(unused_mut)] let mut $s = app.borrow_mut(); $body } }); } }; }
    bind!(on_collection_action, |s, id, action, target| {
        s.begin_collection_action(id, action, target);
    });
    bind!(on_commit_name, |s| {
        s.finish_inline();
    });
    bind!(on_cancel_name, |s| {
        s.cancel_inline();
    });
    bind!(on_finish_delete, |s, index| {
        let id = s.delete_collection_id;
        s.main.set_inline_error("".into());
        let empty = !s.entries.iter().any(|e| e.collection_id == id)
            && !s.collections.iter().any(|c| c.parent_id == Some(id));
        let discard = s.main.get_delete_discard();
        let result = if empty {
            s.store.delete_collection(id)
        } else if discard {
            s.store.delete_collection_with_contents(id)
        } else if let Some(d) = s.main.get_delete_destinations().row_data(index as usize) {
            s.store.migrate_delete(id, i64::from(d.id))
        } else {
            Err(Error::Invalid(
                raptim::i18n::t(
                    "没有可用迁移目标，至少保留一个分组",
                    "No destination available; keep at least one group",
                )
                .into(),
            ))
        };
        match result {
            Ok(()) => {
                s.main.set_collection_delete(false);
                s.report_refresh();
            }
            Err(e) => s.main.set_inline_error(e.to_string().into()),
        }
    });
    bind!(on_begin_conversion, |s, relative| {
        if s.editor.is_some() || !s.finish_inline() {
            s.status(raptim::i18n::t(
                "请先完成项目编辑",
                "Finish editing the item first",
            ));
            return;
        }
        s.scope_selection = s.current_scope();
        s.main.set_conversion_relative(relative);
        s.main.set_conversion_open(true);
        s.update_conversion();
    });
    bind!(on_scope_toggle, |s, id, checked| {
        let ids: Vec<_> = if id < 0 {
            s.collections
                .iter()
                .filter(|c| c.parent_id == Some(-i64::from(id)))
                .map(|c| c.id)
                .collect()
        } else {
            vec![i64::from(id)]
        };
        for id in ids {
            if checked {
                s.scope_selection.insert(id);
            } else {
                s.scope_selection.remove(&id);
            }
        }
        s.update_conversion();
    });
    bind!(on_scope_all, |s, checked| {
        s.scope_selection = if checked {
            s.collections
                .iter()
                .filter(|c| c.parent_id.is_some())
                .map(|c| c.id)
                .collect()
        } else {
            Default::default()
        };
        s.update_conversion();
    });
    bind!(on_apply_conversion, |s| {
        if s.importing || s.editor.is_some() {
            s.main.set_conversion_summary(
                raptim::i18n::t(
                    "请先完成收录或项目编辑",
                    "Finish importing or editing first",
                )
                .into(),
            );
            return;
        }
        let old = s.conversion_preview.clone();
        if let Err(e) = s.refresh() {
            s.main.set_conversion_summary(e.to_string().into());
            return;
        }
        s.update_conversion();
        if old != s.conversion_preview {
            s.main.set_conversion_summary(
                raptim::i18n::t(
                    "数据已变化，预览已更新，请核对后重新转换",
                    "Data changed and preview updated; review before converting",
                )
                .into(),
            );
            return;
        }
        if let Some(preview) = s.conversion_preview.clone() {
            match s.store.convert_entries(&preview) {
                Ok(()) => {
                    s.report_refresh();
                    s.update_conversion();
                    s.main.set_conversion_summary(
                        if raptim::i18n::is_en() {
                            format!(
                                "Converted {} item(s), skipped {} item(s)",
                                preview.entries.len(),
                                preview.skipped.len()
                            )
                        } else {
                            format!(
                                "已转换 {} 项，跳过 {} 项",
                                preview.entries.len(),
                                preview.skipped.len()
                            )
                        }
                        .into(),
                    );
                }
                Err(e) => s.main.set_conversion_summary(e.to_string().into()),
            }
        }
    });
    bind!(on_global_action, |s, action| {
        match action {
            2 | 4 => {
                s.show_settings();
                if s.settings_ui.get().is_none() {
                    return;
                }
                s.settings_ui.set_page(3);
                s.dialog(if action == 2 {
                    DialogKind::ExportJson
                } else {
                    DialogKind::RestoreJson
                });
            }
            5 => {
                if let Err(e) = platform::launch(&Target::Folder {
                    path: s.data_dir.to_string_lossy().into_owned(),
                }) {
                    s.status(e.message());
                }
            }
            6 => {
                s.show_main(ShowSource::Restore);
                s.dock = None;
                s.edge_hold.clear();
                s.sync_dock_overhang();
                platform::restore_position(s.main.window());
                platform::decorate(s.main.window());
            }
            7 => {
                let scope = s.current_scope();
                if let Err(e) = s.store.sort_names(&scope) {
                    s.status(e);
                }
                s.report_refresh();
            }
            8 => s.begin_broken_check(),
            _ => {}
        }
    });
    bind!(on_copy_to, |s, id, target| {
        match s.store.copy_to_collection(i64::from(id), i64::from(target)) {
            Ok(_) => {
                s.report_refresh();
                s.status(raptim::i18n::t(
                    "已复制到所选分类",
                    "Copied to selected category",
                ));
            }
            Err(error) => s.status(error),
        }
    });
    bind!(on_item_action, |s, id, action| {
        if let Some(e) = s.entries.iter().find(|e| e.id == i64::from(id)).cloned() {
            match action {
                0 => {
                    let path = raptim::paths::absolute(e.target.value(), &s.data_dir);
                    if let Err(error) = platform::reveal_file(&path) {
                        s.status(error);
                    }
                }
                1 => {
                    if let Err(error) =
                        arboard::Clipboard::new().and_then(|mut c| c.set_text(e.target.value()))
                    {
                        s.status(error);
                    }
                }
                3 => {
                    s.launch_elevated(i64::from(id));
                }
                7 => {
                    // Opened on purpose from the menu, so there is no failure class to consult;
                    // `show_relocate` still refuses kinds that have no path to re-point.
                    s.show_relocate(
                        i64::from(id),
                        raptim::i18n::t(
                            "重新定位会替换该项目指向的路径，参数、工作目录与窗口方式保持不变。",
                            "Relocating replaces only the target path; arguments, working dir, and run mode stay unchanged.",
                        ),
                        true,
                    );
                }
                4 => {
                    let path = raptim::paths::absolute(e.target.value(), &s.data_dir);
                    let weak = s.main.as_weak();
                    let app_weak = s.weak_self.clone();
                    s.main.set_more_open(true);
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        let open = app_weak
                            .upgrade()
                            .is_none_or(|app| app.borrow_mut().allowed(ShowSource::NativeMenu));
                        if !open {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_more_open(false);
                            }
                            return;
                        }
                        if let Some(ui) = weak.upgrade() {
                            if let Err(error) = platform::shell_context_menu(ui.window(), &path) {
                                ui.set_status(error.into());
                            }
                            ui.window()
                                .dispatch_event(slint::platform::WindowEvent::PointerExited);
                            ui.set_more_open(false);
                        }
                    });
                }
                5 => {
                    let base = s.data_dir.clone();
                    let weak = s.main.as_weak();
                    std::thread::spawn(move || {
                        // `resolve_for_shell`, not `convert_target`: a URL entry has no path to
                        // absolutise, and the shortcut writer handles `url` and AppsFolder targets
                        // itself. Going through the fallible convert failed every 网址 and 应用标识.
                        let target = raptim::paths::resolve_for_shell(&e.target, &base);
                        let result = platform::desktop_shortcut(&e.name, &target);
                        let message = result
                            .map(|path| {
                                format!(
                                    "{}: {}",
                                    raptim::i18n::t("已创建", "Created"),
                                    path.display()
                                )
                            })
                            .unwrap_or_else(|e| e);
                        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(message.into()));
                    });
                }
                6 => {
                    let base = s.data_dir.clone();
                    let sink = s.events.clone();
                    let epoch = s.data_epoch;
                    std::thread::spawn(move || {
                        let new_icon = platform::icon_for(&e.target, &base);
                        sink.send(AppEvent::RefreshIconReady {
                            epoch,
                            id: e.id,
                            key: e.target.duplicate_key(),
                            old_icon: e.icon,
                            new_icon,
                        });
                    });
                }
                _ => {}
            }
        }
    });
    main.on_nav_drag(|id| {
        let mut data = slint::DataTransfer::default();
        data.set_user_data(Rc::new(DraggedCollection(i64::from(id))));
        data
    });
    let weak = Rc::downgrade(app);
    main.on_nav_can_drop(move |data, id| {
        weak.upgrade().is_some_and(|app| {
            let s = app.borrow();
            if s.editor.is_some() || s.main.get_busy() {
                return false;
            }
            if dragged_id(&data).is_some() {
                return true;
            }
            let Some(source) = data
                .user_data()
                .and_then(|v| v.downcast_ref::<DraggedCollection>().map(|v| v.0))
            else {
                return false;
            };
            s.collections
                .iter()
                .find(|c| c.id == source)
                .is_some_and(|source| {
                    s.collections
                        .iter()
                        .find(|c| c.id == i64::from(id))
                        .is_some_and(|dest| {
                            source.parent_id == dest.parent_id
                                || (source.parent_id.is_some() && dest.parent_id.is_none())
                        })
                })
        })
    });
    let weak = Rc::downgrade(app);
    main.on_nav_drop(move |data, id, after| {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        let mut s = app.borrow_mut();
        if s.editor.is_some() || s.main.get_busy() {
            return false;
        }
        let destination = if id <= 0 { s.group } else { i64::from(id) };
        let result = if let Some(source) = dragged_id(&data) {
            let category = s.category_for(destination);
            s.store.move_to_collection(source, category)
        } else if let Some(source) = data
            .user_data()
            .and_then(|v| v.downcast_ref::<DraggedCollection>().map(|v| v.0))
        {
            if let (Some(c), Some(d)) = (
                s.collections.iter().find(|c| c.id == source).cloned(),
                s.collections.iter().find(|c| c.id == destination).cloned(),
            ) {
                if c.parent_id == d.parent_id {
                    s.store.reorder_collection_at(source, destination, after)
                } else if c.parent_id.is_some() && d.parent_id.is_none() {
                    s.store
                        .save_collection(Some(source), Some(destination), &c.name)
                        .map(|_| ())
                } else {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            return false;
        };
        match result {
            Ok(()) => {
                s.report_refresh();
                true
            }
            Err(e) => {
                s.status(e);
                false
            }
        }
    });
}
