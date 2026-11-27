// Window lifecycle and browsing state helpers for `App`.
use super::*;

impl App {
    pub(super) fn restore_protected_edge(&mut self) {
        // Re-checked here: this collapses the window onto the edge strip and raises it, which is
        // still a covering topmost. Returning before `take` keeps the resume snapshot for a later,
        // genuine release (Strict -> Fullscreen is a de-escalation but not a release).
        if self.policy.protection() != Protection::Normal {
            return;
        }
        let Some(state) = self
            .protection_resume
            .take(self.settings.edge_hide, self.main.get_pinned())
        else {
            return;
        };
        platform::restore_saved(self.main.window(), &state);
        let Some((rect, work)) = platform::geometry(self.main.window()) else {
            return;
        };
        let Some(edge) = state.edge.and_then(|e| match e {
            0 => Some(raptim::docking::Edge::Left),
            1 => Some(raptim::docking::Edge::Right),
            2 => Some(raptim::docking::Edge::Top),
            _ => None,
        }) else {
            return;
        };
        self.dock = Some((edge, rect));
        self.dock_work = Some(work);
        self.edge_motion = None;
        self.edge_hidden = true;
        self.edge_hold.clear();
        self.protected_edge_waiting = true;
        self.edge_progress = 1.;
        self.focus_hide.reset();
        self.edge_since = Instant::now();
        self.sync_dock_overhang();
        let strip = self.edge_strip_px();
        let pad = self.dock_overhang_for(edge);
        platform::move_clipped(
            self.main.window(),
            raptim::docking::hidden_hwnd_position(edge, rect, strip, pad),
            work,
        );
        self.apply_main_topmost(true);
        self.sync_edge_appearance();
        if let Err(e) = platform::show_without_activation(self.main.window()) {
            self.status(e);
            return;
        }
        self.apply_theme();
        windowing::invalidate(self.main.window());
        self.schedule_interaction();
        self.sync_poll_timer();
    }

    pub(super) fn install_editor_drop(&mut self, generation: u64) {
        let Some(session) = self.editor.as_ref().filter(|s| s.generation == generation) else {
            return;
        };
        if session._drop.is_some()
            || session.drop_failed
            || !platform::native_ready(session.window.window())
        {
            return;
        }
        let sink = self.events.clone();
        let result =
            platform::drop_target::Registration::new(session.window.window(), move |event| {
                if let platform::drop_target::DropEvent::Data(
                    raptim::importing::ExternalDrop::Files(paths),
                    _,
                    _,
                ) = event
                    && paths.len() == 1
                {
                    sink.send(AppEvent::EditorImageDropped {
                        generation,
                        path: paths[0].to_string_lossy().into_owned(),
                    });
                }
            });
        if let Some(session) = self.editor.as_mut().filter(|s| s.generation == generation) {
            match result {
                Ok(registration) => {
                    session._drop = Some(registration);
                }
                Err(error) => {
                    session.drop_failed = true;
                    session.window.set_error(
                        format!(
                            "{}: {error}",
                            raptim::i18n::t("图片拖入不可用", "Image drop unavailable")
                        )
                        .into(),
                    );
                }
            }
        }
    }
    pub(super) fn initialize_native_state(&mut self) {
        // The first show can precede winit's resumed event. Do not convert saved
        // DIP sizes or register OLE until the native window and its DPI exist.
        if !self.main.window().is_visible() || platform::geometry(self.main.window()).is_none() {
            return;
        }
        if !self.position_restored {
            platform::restore_saved(self.main.window(), &self.saved_window);
            if self.settings.edge_hide
                && let Some((r, work)) = platform::geometry(self.main.window())
            {
                self.dock = raptim::docking::dock(r, work, 0);
                self.dock_work = Some(work);
                self.sync_dock_overhang();
                if self.dock.is_some() && !self.edge_hidden {
                    self.edge_hold
                        .arm(Instant::now(), platform::app_has_focus());
                }
            }
            self.position_restored = true;
            self.apply_main_topmost(self.edge_motion.is_some() || self.wants_topmost());
            self.apply_theme();
        }
        self.install_external_drop();
    }

    pub(super) fn install_external_drop(&mut self) {
        if self.external_drop.is_some() {
            return;
        }
        let weak = self.main.as_weak();
        let sink = self.events.clone();
        match platform::drop_target::Registration::new(self.main.window(), move |event| {
            use platform::drop_target::DropEvent;
            let sink = sink.clone();
            let _ = weak.upgrade_in_event_loop(move |ui| match event {
                DropEvent::Hover(x, y) | DropEvent::Data(_, x, y) => {
                    if let Some((r, _)) = platform::geometry(ui.window()) {
                        ui.set_external_x((x - r.x) as f32 / ui.window().scale_factor());
                        ui.set_external_y((y - r.y) as f32 / ui.window().scale_factor());
                    }
                    ui.set_drop_hover(true);
                    if let DropEvent::Data(value, _, _) = event {
                        slint::Timer::single_shot(Duration::ZERO, move || {
                            sink.send(AppEvent::ExternalDrop(value));
                        });
                    }
                }
                DropEvent::Leave => ui.set_drop_hover(false),
            });
        }) {
            Ok(r) => self.external_drop = Some(r),
            Err(e) => self.status(format!(
                "{}: {e}",
                raptim::i18n::t("外部拖入不可用", "External drop unavailable")
            )),
        }
    }

    pub(super) fn handle_external_drop(&mut self, drop_data: raptim::importing::ExternalDrop) {
        if !self.allowed(ShowSource::Editor) {
            self.main.set_drop_hover(false);
            return;
        }
        let destination = if self.main.get_external_category() != 0 {
            self.category_for(i64::from(self.main.get_external_category()))
        } else {
            self.collection
        };
        self.main.set_drop_hover(false);
        match drop_data {
            raptim::importing::ExternalDrop::Files(paths) => {
                if paths.is_empty() {
                    self.status(raptim::i18n::t(
                        "每次最多添加 500 项",
                        "You can add at most 500 items at once",
                    ));
                } else if let Err(e) = self.queue_files(paths, destination) {
                    self.status(e);
                }
            }
            raptim::importing::ExternalDrop::Text(text) => {
                match raptim::importing::url_entries(&text, destination)
                    .and_then(|entries| self.store.add_entries(&entries))
                {
                    Ok((added, duplicates)) => {
                        self.status(if raptim::i18n::is_en() {
                            format!("Added {added} item(s), skipped {duplicates} duplicate(s)")
                        } else {
                            format!("已添加 {added} 项，跳过 {duplicates} 项重复目标")
                        });
                        self.report_refresh();
                        let wanted: Vec<_> = self
                            .entries
                            .iter()
                            .filter(|e| e.icon.is_empty() && matches!(e.target, Target::Url { .. }))
                            .map(|e| (e.id, e.target.clone()))
                            .collect();
                        self.fill_icons(wanted);
                    }
                    Err(e) => self.status(e),
                }
            }
        }
    }

    /// A backfilled or refreshed icon replaces one blob in place, then asks the band sweep to
    /// re-decode just that row. The previous shape ran `save_entry` plus a full `refresh()` per
    /// reply, so a 300-row startup backfill re-read the whole library about twenty times right
    /// after the first paint.
    pub(super) fn replace_icon(&mut self, id: i64, icon: Vec<u8>) -> bool {
        let index = match self.icon_index.get(&id) {
            Some(key) if self.entries.get(key.index).is_some_and(|e| e.id == id) => key.index,
            _ => return false,
        };
        if let Err(error) = self.store.save_icon(id, &icon) {
            self.status(error);
            return false;
        }
        self.entries[index].icon = icon;
        self.store.clear_dirty_icon(id);
        // The sweep keys its cache on the blob's fingerprint, so it has to follow the new bytes.
        if let Some(key) = self.icon_index.get_mut(&id) {
            key.fingerprint = raptim::icons::fingerprint(&self.entries[index].icon);
        }
        self.icon_cache.remove(&id);
        true
    }
    pub(super) fn update_visible_icons(&mut self) {
        let mut visible = std::collections::HashSet::new();
        let mut bands = vec![(
            self.main.get_items(),
            &self.grid_ids[..],
            self.main.get_grid_first(),
            self.main.get_grid_visible_count().max(36),
        )];
        if self.main.get_search_open() {
            bands.push((
                self.main.get_results(),
                &self.result_ids[..],
                self.main.get_search_first(),
                self.main.get_search_visible_count().max(12),
            ));
        }
        // `grid_first` can overshoot `row_count` right after a rebuild, and unlike `row_data` a
        // slice index would panic on it.
        let bands: Vec<_> = bands
            .into_iter()
            .map(|(model, ids, first, count)| {
                let start = (first.max(0) as usize).min(ids.len());
                let end = (start + count.min(256) as usize).min(ids.len());
                (model, ids, start, end)
            })
            .collect();
        for (_, ids, start, end) in &bands {
            for id in &ids[*start..*end] {
                if visible.len() < 224 {
                    visible.insert(*id);
                }
            }
        }
        for &id in &visible {
            if let Some(&raptim::icons::IconKey { fingerprint, index }) = self.icon_index.get(&id) {
                self.icon_cache
                    .request(id, fingerprint, &self.entries[index].icon);
            }
        }
        let changed: std::collections::HashSet<_> = self
            .icon_cache
            .drain(&self.icon_index, &visible)
            .into_iter()
            .collect();
        if visible == self.visible_icons && changed.is_empty() {
            return;
        }
        // Rows hold their icon reference until they leave the band, so the sweep covers the
        // previous band too.
        let previous = std::mem::take(&mut self.icon_bands);
        for (index, (model, ids, start, end)) in bands.iter().enumerate() {
            // A band with no history means its model was just rebuilt: `display_item` ran before
            // `visible_icons` was cleared, so those rows carry icons from the old band and every
            // row in range has to be rewritten.
            let tracked = previous.get(index).is_some();
            let (was_start, was_end) = previous.get(index).copied().unwrap_or((*start, *end));
            for i in (*start).min(was_start)..(*end).max(was_end) {
                let Some(&id) = ids.get(i) else {
                    continue;
                };
                let now_in = (*start..*end).contains(&i) && visible.contains(&id);
                let was_in = (was_start..was_end).contains(&i) && self.visible_icons.contains(&id);
                if tracked && now_in == was_in && !changed.contains(&id) {
                    continue;
                }
                if let Some(mut item) = model.row_data(i) {
                    let (icon, drag) = if now_in {
                        self.icon_cache.get(id)
                    } else {
                        Default::default()
                    };
                    if item.icon != icon || item.drag_icon != drag {
                        item.icon = icon;
                        item.drag_icon = drag;
                        model.set_row_data(i, item);
                    }
                }
            }
            self.icon_bands.push((*start, *end));
        }
        self.visible_icons = visible;
    }
    pub(super) fn schedule_interaction(&self) {
        let edge_animation = self.main.window().is_visible() && self.edge_motion.is_some();
        self.frame_clock.set_active(edge_animation);
        if edge_animation {
            self.interaction_timer.stop();
            return;
        }
        let Some(delay) = raptim::session::interaction_interval(
            self.main.window().is_visible(),
            // The focus fade only runs in the undocked branch of tick_window, so a docked
            // launcher must not ask for the animation rate just because it lacks focus.
            self.dock.is_none() && self.focus_hide_active(),
            self.dock.is_some() || self.drag.is_some(),
            self.collapsed_far,
        ) else {
            self.interaction_timer.stop();
            return;
        };
        // A launcher drag reaches this on every Moved event. A timer already counting down at
        // the same interval needs no teardown; inside its own callback `running()` is already
        // false, so the self-rearm below still goes through.
        let delay = Duration::from_millis(delay);
        if self.interaction_timer.running() && self.interaction_timer.interval() == delay {
            return;
        }
        self.interaction_timer.stop();
        let weak = self.weak_self.clone();
        self.interaction_timer
            .start(slint::TimerMode::SingleShot, delay, move || {
                if let Some(app) = weak.upgrade() {
                    let mut app = app.borrow_mut();
                    if !app.collapsed_probe() {
                        app.tick_window(true);
                    }
                    app.schedule_interaction();
                }
            });
    }
    pub(super) fn focus_hide_active(&self) -> bool {
        self.settings.hide_on_focus_loss
            && !platform::app_has_focus()
            && !self.auto_hide_guard().paused()
    }
    pub(super) fn save_window(&mut self, force: bool) {
        if (!force && (self.drag.is_some() || self.edge_motion.is_some()))
            || !self.main.window().is_visible()
        {
            return;
        }
        let Some((actual, _)) = platform::geometry(self.main.window()) else {
            return;
        };
        let rect = self.dock.map_or(actual, |(_, r)| r);
        let scale = self.main.window().scale_factor();
        let candidate = raptim::session::WindowState {
            always_on_top: self.saved_window.always_on_top,
            x: rect.x,
            y: rect.y,
            width: rect.width as f32 / scale,
            height: rect.height as f32 / scale,
            monitor: platform::monitor_name(self.main.window()),
            edge: self.dock.map(|(e, _)| match e {
                raptim::docking::Edge::Left => 0,
                raptim::docking::Edge::Right => 1,
                raptim::docking::Edge::Top => 2,
            }),
            group: self.group,
            category: self.collection,
        };
        if candidate == self.saved_window {
            self.window_candidate = None;
            return;
        }
        if !force
            && self
                .window_candidate
                .as_ref()
                .is_none_or(|(old, _)| old != &candidate)
        {
            self.window_candidate = Some((candidate, Instant::now()));
            return;
        }
        if force
            || self
                .window_candidate
                .as_ref()
                .is_some_and(|(_, since)| since.elapsed() >= Duration::from_millis(500))
        {
            match self.store.save_window_state(&candidate) {
                Ok(()) => {
                    self.saved_window = candidate;
                    self.window_candidate = None;
                }
                Err(e) => self.status(e),
            }
        }
    }
    pub(super) fn remember_view(&mut self) {
        let focus = self.main.get_selected_id();
        let index = self
            .main
            .get_items()
            .iter()
            .position(|e| e.id == focus)
            .unwrap_or(0);
        self.browsing
            .views
            .insert(self.collection, (self.main.get_grid_scroll(), focus, index));
        self.browsing
            .group_categories
            .insert(self.group, self.collection);
    }
    pub(super) fn restore_view(&self) {
        let (scroll, focus, index) = self
            .browsing
            .views
            .get(&self.collection)
            .copied()
            .unwrap_or_default();
        self.main.set_grid_scroll(scroll);
        let items = self.main.get_items();
        let id = if focus == 0 {
            0
        } else if items.iter().any(|e| e.id == focus) {
            focus
        } else {
            items
                .row_data(index.min(items.row_count().saturating_sub(1)))
                .map_or(0, |e| e.id)
        };
        self.main.set_selected_id(id);
    }
    pub(super) fn displayed_ids(&self) -> Vec<i64> {
        let model = if self.main.get_search_open() {
            self.main.get_results()
        } else {
            self.main.get_items()
        };
        model.iter().map(|e| i64::from(e.id)).collect()
    }
    pub(super) fn clear_batch(&mut self) {
        self.reset_batch_selection();
        self.main.set_batch_mode(false);
    }
    pub(super) fn reset_batch_selection(&mut self) {
        // Every search keystroke arrives here. With nothing selected there is no row left to
        // un-check, and walking both models cloned every row of both just to re-derive the two
        // counters that are set a few lines below anyway.
        let row_flags = !self.browsing.selected.is_empty() || self.main.get_batch_count() != 0;
        self.browsing.clear_selection();
        self.main.set_batch_count(0);
        self.main.set_batch_confirm(false);
        self.main.set_batch_target_open(false);
        self.main.set_batch_error("".into());
        self.main.set_batch_all_selected(false);
        if row_flags {
            self.update_selection();
        }
    }
    pub(super) fn update_selection(&self) {
        for model in [self.main.get_items(), self.main.get_results()] {
            for i in 0..model.row_count() {
                if let Some(mut item) = model.row_data(i) {
                    let checked = self.browsing.selected.contains(&i64::from(item.id));
                    if item.checked != checked {
                        item.checked = checked;
                        model.set_row_data(i, item);
                    }
                }
            }
        }
        self.main
            .set_batch_count(self.browsing.selected.len() as i32);
        let ids = self.displayed_ids();
        self.main.set_batch_all_selected(
            !ids.is_empty() && ids.iter().all(|id| self.browsing.selected.contains(id)),
        );
    }
    fn build_destination_nodes(
        &self,
        expanded: &std::collections::HashSet<i64>,
    ) -> Vec<DestinationNode> {
        let mut nodes = Vec::new();
        for group in self.collections.iter().filter(|c| c.parent_id.is_none()) {
            let is_expanded = expanded.contains(&group.id);
            nodes.push(DestinationNode {
                id: group.id as i32,
                parent: 0,
                name: group.name.clone().into(),
                expanded: is_expanded,
            });
            if is_expanded {
                nodes.extend(
                    self.collections
                        .iter()
                        .filter(|c| c.parent_id == Some(group.id))
                        .map(|c| DestinationNode {
                            id: c.id as i32,
                            parent: group.id as i32,
                            name: c.name.clone().into(),
                            expanded: false,
                        }),
                );
            }
        }
        nodes
    }
    fn batch_tree(&self) {
        let nodes = self.build_destination_nodes(&self.batch_expanded);
        self.main
            .set_batch_nodes(Rc::new(VecModel::from(nodes)).into());
    }
    /// The same group/category tree the organizer uses, driven by the editor's own expansion set.
    fn collection_tree(&mut self) {
        let nodes = self.build_destination_nodes(&self.editor_expanded);
        let count = nodes.len() as i32;
        let editor = match self.editor.as_ref() {
            Some(session) => session.window.clone_strong(),
            None => return,
        };
        editor.set_collection_nodes(Rc::new(VecModel::from(nodes)).into());
        editor.set_collection_cursor(editor.get_collection_cursor().clamp(0, (count - 1).max(0)));
    }
    pub(super) fn pick_collection(&mut self) {
        let current = self.collections.iter().find(|c| {
            c.id == i64::from(
                self.editor
                    .as_ref()
                    .map_or(0, |e| e.window.get_collection_id()),
            )
        });
        self.editor_expanded.clear();
        if let Some(category) = current
            && let Some(parent) = category.parent_id
        {
            self.editor_expanded.insert(parent);
        } else if let Some(group) = current {
            self.editor_expanded.insert(group.id);
        }
        if let Some(session) = self.editor.as_ref() {
            session.window.set_collection_open(true);
        }
        self.collection_tree();
        if let Some(session) = self.editor.as_ref() {
            let selected = session.window.get_collection_id();
            let index = session
                .window
                .get_collection_nodes()
                .iter()
                .position(|n| n.id == selected)
                .unwrap_or(0) as i32;
            session.window.set_collection_cursor(index);
            session.window.invoke_focus_collection_tree();
        }
    }
    pub(super) fn collection_node(&mut self, id: i32) {
        let Some(session) = self.editor.as_ref() else {
            return;
        };
        let id = i64::from(id);
        let window = session.window.clone_strong();
        let Some(collection) = self.collections.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        if collection.parent_id.is_none() {
            if !self.editor_expanded.remove(&id) {
                self.editor_expanded.insert(id);
            }
            self.collection_tree();
            return;
        }
        window.set_collection_id(id as i32);
        window.set_collection_name(self.collection_name(id));
        window.set_collection_open(false);
        window.invoke_focus_name();
    }
    pub(super) fn collection_key(&mut self, direction: i32) {
        let Some(session) = self.editor.as_ref() else {
            return;
        };
        let window = session.window.clone_strong();
        let nodes = window.get_collection_nodes();
        let Some(step) = step_destination_tree(&nodes, window.get_collection_cursor(), direction)
        else {
            return;
        };
        match step {
            TreeKeyStep::Cursor { next, .. } => window.set_collection_cursor(next),
            TreeKeyStep::ToggleGroup(id) | TreeKeyStep::ActivateLeaf(id) => {
                self.collection_node(id);
            }
        }
    }
    /// Filtered list handed to the picker. Filtering stays here so the view only renders the
    /// result rather than testing every installed app on each keystroke.
    pub(super) fn refresh_app_list(&mut self) {
        let Some(session) = &self.editor else {
            return;
        };
        let filter = session.window.get_picker_filter().to_lowercase();
        let shown: Vec<AppOption> = self
            .installed_apps
            .iter()
            .filter(|(aumid, name)| {
                filter.is_empty()
                    || name.to_lowercase().contains(&filter)
                    || aumid.to_lowercase().contains(&filter)
            })
            .map(|(aumid, name)| AppOption {
                name: name.clone().into(),
                aumid: aumid.clone().into(),
            })
            .collect();
        session
            .window
            .set_picker_shown(Rc::new(VecModel::from(shown)).into());
    }
    pub(super) fn open_app_picker(&mut self) {
        if !self.allowed(ShowSource::Editor) {
            return;
        }
        let Some(session) = &self.editor else {
            return;
        };
        session.window.set_picker_filter("".into());
        session.window.set_picker_aumid("".into());
        session.window.set_picker_name("".into());
        self.refresh_app_list();
        let Some(session) = &self.editor else {
            return;
        };
        session.window.set_picker_open(true);
        session.window.invoke_focus_picker_filter();
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let apps = platform::installed_apps();
            sink.send(AppEvent::InstalledAppsReady(apps));
        });
    }
    pub(super) fn close_app_picker(&mut self) {
        let Some(session) = &self.editor else {
            return;
        };
        if !session.window.get_picker_open() {
            return;
        }
        session.window.set_picker_open(false);
        self.focus_editor_after_picker();
    }
    /// Dismissing the picker must leave the caret in the editor, not on the desktop.
    fn focus_editor_after_picker(&self) {
        if let Some(session) = &self.editor {
            platform::focus(session.window.window());
        }
    }
    pub(super) fn choose_app(&mut self, aumid: String, name: String) {
        self.close_app_picker();
        let Some(session) = &self.editor else {
            return;
        };
        session.window.set_target(aumid.into());
        if session.window.get_entry_name().trim().is_empty() {
            session.window.set_entry_name(name.into());
        }
        // Only a form that still has no icon gets the preview; a picked custom icon stays picked.
        let preview = session.icon.is_empty();
        if preview {
            self.extract_editor_icon();
        }
    }
    pub(super) fn batch_target(&mut self, action: i32) {
        if self.browsing.selected.is_empty() || !self.main.get_batch_mode() {
            return;
        }
        self.batch_expanded.clear();
        self.batch_expanded.insert(self.group);
        self.main.set_batch_action(action);
        self.main.set_batch_destination(0);
        self.main.set_batch_target_name("".into());
        self.main.set_batch_error("".into());
        self.batch_tree();
        self.main.set_batch_tree_index(
            self.main
                .get_batch_nodes()
                .iter()
                .position(|n| n.id == self.group as i32)
                .unwrap_or(0) as i32,
        );
        self.main.set_batch_target_open(true);
        self.main.invoke_focus_batch_tree();
    }
    pub(super) fn batch_node(&mut self, id: i32) {
        let Some(node) = self.collections.iter().find(|n| n.id == i64::from(id)) else {
            return;
        };
        if node.parent_id.is_none() {
            if !self.batch_expanded.remove(&node.id) {
                self.batch_expanded.insert(node.id);
            }
            self.batch_tree();
        } else {
            self.main.set_batch_destination(id);
            self.main
                .set_batch_target_name(self.collection_name(i64::from(id)));
            self.main.set_batch_error("".into());
        }
        self.main.set_batch_tree_index(
            self.main
                .get_batch_nodes()
                .iter()
                .position(|n| n.id == id)
                .unwrap_or(0) as i32,
        );
    }
    pub(super) fn batch_tree_key(&mut self, direction: i32) {
        let nodes = self.main.get_batch_nodes();
        let Some(step) = step_destination_tree(&nodes, self.main.get_batch_tree_index(), direction)
        else {
            return;
        };
        match step {
            TreeKeyStep::Cursor { next, row_id, leaf } => {
                self.main.set_batch_tree_index(next);
                if leaf && matches!(direction, -1 | 1 | 2) {
                    self.batch_node(row_id);
                }
            }
            TreeKeyStep::ToggleGroup(id) => self.batch_node(id),
            TreeKeyStep::ActivateLeaf(id) => {
                self.batch_node(id);
                self.batch_action(self.main.get_batch_action(), id);
            }
        }
    }
    pub(super) fn batch_action(&mut self, action: i32, destination: i32) {
        if !self.main.get_batch_mode()
            || self.browsing.selected.is_empty()
            || !(0..=2).contains(&action)
        {
            return;
        }
        if self.editor.is_some() || self.importing || self.file_dialog_open {
            self.main.set_batch_error(
                raptim::i18n::t(
                    "请先完成编辑或收录",
                    "Finish editing or importing items first",
                )
                .into(),
            );
            return;
        }
        let ids: Vec<_> = self
            .displayed_ids()
            .into_iter()
            .filter(|id| self.browsing.selected.contains(id))
            .collect();
        if ids.len() != self.browsing.selected.len() {
            self.main.set_batch_error(
                raptim::i18n::t(
                    "项目已变化，请重新选择",
                    "Items changed; please select again",
                )
                .into(),
            );
            return;
        }
        let dest = (action != 2).then_some(i64::from(destination));
        match self.store.batch_entries(&ids, dest, action == 1) {
            Ok(()) => {
                self.remember_view();
                self.reset_batch_selection();
                self.report_refresh();
                self.update_selection();
                self.main.invoke_focus_browser();
                self.status(if raptim::i18n::is_en() {
                    format!("Processed {} item(s)", ids.len())
                } else {
                    format!("已处理 {} 项", ids.len())
                });
            }
            Err(e) => self.main.set_batch_error(e.to_string().into()),
        }
    }
}

enum TreeKeyStep {
    Cursor { next: i32, row_id: i32, leaf: bool },
    ToggleGroup(i32),
    ActivateLeaf(i32),
}

fn step_destination_tree(
    nodes: &ModelRc<DestinationNode>,
    cursor: i32,
    direction: i32,
) -> Option<TreeKeyStep> {
    let index = cursor.max(0) as usize;
    let node = nodes.row_data(index)?;
    let last = i32::try_from(nodes.row_count())
        .unwrap_or(1)
        .saturating_sub(1);
    match direction {
        -1 | 1 => {
            let next = (cursor + direction).clamp(0, last);
            let target = nodes.row_data(next as usize)?;
            Some(TreeKeyStep::Cursor {
                next,
                row_id: target.id,
                leaf: target.parent != 0,
            })
        }
        2 if node.parent == 0 => {
            if node.expanded {
                step_destination_tree(nodes, index as i32, 1)
            } else {
                Some(TreeKeyStep::ToggleGroup(node.id))
            }
        }
        -2 if node.parent != 0 => nodes
            .iter()
            .position(|n| n.id == node.parent)
            .map(|parent| TreeKeyStep::Cursor {
                next: parent as i32,
                row_id: node.parent,
                leaf: false,
            }),
        -2 if node.expanded => Some(TreeKeyStep::ToggleGroup(node.id)),
        0 if node.parent == 0 => Some(TreeKeyStep::ToggleGroup(node.id)),
        0 => Some(TreeKeyStep::ActivateLeaf(node.id)),
        _ => None,
    }
}
