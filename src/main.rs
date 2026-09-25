#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use diascia::{
    launch::Failure,
    model::*,
    platform,
    policy::{DisplayPolicy, Protection, ShowSource, decide},
    storage::Store,
    windowing,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
slint::include_modules!();
include!("ui_commands.rs");
include!("app_session.rs");
#[cfg(test)]
mod app_regression;

struct EditorSession {
    _drop: Option<platform::drop_target::Registration>,
    drop_failed: bool,
    generation: u64,
    window: EditorWindow,
    original: Entry,
    icon: Vec<u8>,
    icon_attempted: bool,
    collections: Vec<i64>,
}
#[derive(Clone, Copy)]
struct DraggedEntry(i64);
#[derive(Clone, Copy)]
struct DraggedCollection(i64);
fn dragged_id(data: &slint::DataTransfer) -> Option<i64> {
    data.user_data()?
        .downcast_ref::<DraggedEntry>()
        .map(|v| v.0)
}
struct App {
    protection_resume: diascia::session::ProtectionResume,
    editor_generation: u64,
    batch_expanded: std::collections::HashSet<i64>,
    editor_expanded: std::collections::HashSet<i64>,
    external_drop: Option<platform::drop_target::Registration>,
    main: MainWindow,
    weak_self: std::rc::Weak<RefCell<App>>,
    interaction_timer: slint::Timer,
    frame_clock: platform::FrameClock,
    position_restored: bool,
    store: Store,
    data_dir: PathBuf,
    collections: Vec<Collection>,
    entries: Vec<Entry>,
    settings: Settings,
    group: i64,
    collection: i64,
    policy: DisplayPolicy,
    protection_cache: platform::ProtectionCache,
    native_epoch: Arc<AtomicU64>,
    hotkey: platform::Hotkey,
    _foreground: platform::ForegroundWatch,
    tray: platform::Tray,
    last_evaluated: Instant,
    notices: std::cell::RefCell<diascia::session::NoticeQueue>,
    _instance: platform::Instance,
    initial: Option<Export>,
    data_epoch: u64,
    saved_window: diascia::session::WindowState,
    window_candidate: Option<(diascia::session::WindowState, Instant)>,
    browsing: diascia::session::Browsing,
    editor: Option<EditorSession>,
    settings_ui: Deferred<SettingsWindow>,
    settings_open: bool,
    installed_apps: Vec<(String, String)>,
    drag: Option<((i32, i32), (i32, i32))>,
    dock: Option<(diascia::docking::Edge, windowing::Rect)>,
    edge_hidden: bool,
    /// Set by `collapsed_probe` when the pointer is outside the approach margin, so the next
    /// interaction tick can run at half rate.
    collapsed_far: bool,
    protected_edge_waiting: bool,
    dock_work: Option<windowing::Rect>,
    edge_progress: f32,
    edge_motion: Option<diascia::docking::Motion>,
    motion_frame: Option<Instant>,
    focus_hide: diascia::docking::FocusHide,
    edge_since: Instant,
    search_index: diascia::search::SearchIndex,
    icon_cache: diascia::icons::Cache,
    icon_index: std::collections::HashMap<i64, diascia::icons::IconKey>,
    visible_icons: std::collections::HashSet<i64>,
    items_signature: Vec<ItemSignature>,
    results_signature: Vec<ItemSignature>,
    /// Row ids parallel to the `items` and `results` models, so the viewport sweep can read an id
    /// without cloning a whole row. Rebuilt exactly when the matching model is.
    grid_ids: Vec<i64>,
    result_ids: Vec<i64>,
    icon_bands: Vec<(usize, usize)>,
    /// The on-screen collection's ids. `on_can_drop` fires on every mouse-move of a reorder drag.
    collection_ids: std::collections::HashSet<i64>,
    collection_names: std::collections::HashMap<i64, SharedString>,
    /// Search scope results keyed by (scope, group, collection); only `collections` can change it
    /// underneath, and that happens in `refresh`.
    eligible: Option<((i32, i64, i64), std::collections::HashSet<i64>)>,
    scope_selection: std::collections::HashSet<i64>,
    conversion_preview: Option<diascia::commands::ConversionPreview>,
    delete_collection_id: i64,
    /// Entry the relocate panel is pointing at; 0 when nothing is armed. Kept on the Rust side
    /// so the panel's display properties stay strings and no id crosses the Slint int boundary.
    relocate_pending: i64,
    search_was_open: bool,
    pending_import: Option<Export>,
    quitting: bool,
    file_dialog_open: bool,
    dialog_collection: i64,
    pending_files: Vec<(PathBuf, i64)>,
    last_drop: Instant,
    importing: bool,
    icon_epoch: u64,
    icon_pending: bool,
    saving_editor: bool,
}
fn strings(values: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        values
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
    .into()
}
/// Everything a row displays except the icon, which the icon cache owns separately.
type ItemSignature = (
    i32,
    bool,
    SharedString,
    SharedString,
    SharedString,
    SharedString,
    i32,
);
fn item_signatures(items: &[Item]) -> Vec<ItemSignature> {
    items
        .iter()
        .map(|i| {
            (
                i.id,
                i.checked,
                i.name.clone(),
                i.detail.clone(),
                i.tooltip_path.clone(),
                i.tooltip_notes.clone(),
                i.kind,
            )
        })
        .collect()
}
impl App {
    fn wants_topmost(&self) -> bool {
        self.policy.protection() == Protection::Normal
            && (self.saved_window.always_on_top || self.main.get_pinned())
    }
    fn auto_hide_guard(&self) -> diascia::policy::AutoHideGuard {
        diascia::policy::AutoHideGuard {
            editor: self
                .editor
                .as_ref()
                .is_some_and(|e| e.window.window().is_visible()),
            settings: self
                .settings_ui
                .get()
                .is_some_and(|ui| ui.window().is_visible()),
            name: self.main.get_inline_kind() != 0,
            file_dialog: self.file_dialog_open,
            pinned: self.main.get_pinned(),
            interaction: self.main.get_interacting()
                || self.main.get_drop_hover()
                || self.drag.is_some()
                || windowing::has_popup(self.main.window()),
        }
    }
    /// Appearance only: un-fade and slide the window back to its docked rectangle. Refuses to
    /// touch the window while any protection holds, so a background tick can never reveal it.
    fn reveal_edge(&mut self) {
        if self.policy.protection() != Protection::Normal {
            return;
        }
        let displaced = self.edge_hidden || self.edge_progress > 0.;
        self.edge_motion = None;
        self.motion_frame = None;
        self.edge_progress = 0.;
        self.focus_hide.reset();
        platform::set_opacity(self.main.window(), 1.);
        if displaced && let Some((_, r)) = self.dock {
            self.main
                .window()
                .set_position(slint::PhysicalPosition::new(r.x, r.y));
            platform::decorate(self.main.window());
            windowing::invalidate(self.main.window());
            platform::topmost(self.main.window(), self.wants_topmost());
        }
        self.edge_hidden = false;
    }
    /// A user-initiated summon. Clearing `protected_edge_waiting` means "the collapsed recovery
    /// state is consumed", which only this path is allowed to decide.
    fn user_summon_edge(&mut self) {
        self.protected_edge_waiting = false;
        self.reveal_edge();
        self.edge_since = Instant::now();
    }
    /// Deterministic collapsed state, used when protection interrupts an in-flight slide.
    fn settle_edge_hidden(&mut self) {
        self.edge_motion = None;
        self.motion_frame = None;
        self.edge_progress = 1.;
        self.edge_hidden = true;
    }
    /// Applies the current temporary pin, then refreshes the tray check marks.
    fn apply_pin(&mut self) {
        self.user_summon_edge();
        platform::topmost(self.main.window(), self.wants_topmost());
        self.sync_tray_states();
    }
    fn sync_tray_states(&self) {
        self.tray
            .set_states(self.main.get_pinned(), self.policy.bypass());
    }
    /// Manual override for one run only: the hotkey, tray and edge entries obey it, and it is
    /// never written to the database so a launcher cannot be left invisible over a game.
    fn toggle_protection_bypass(&mut self) {
        self.policy.set_bypass(!self.policy.bypass());
        self.evaluate();
        self.sync_tray_states();
        self.sync_settings_ui();
        self.status(if self.policy.bypass() {
            "已暂停游戏勿扰，仅本次运行有效"
        } else {
            "已恢复游戏勿扰判定"
        });
    }
    fn animate_edge(&mut self, hidden: bool) {
        if self.policy.protection() != Protection::Normal {
            self.settle_edge_hidden();
            return;
        }
        self.protected_edge_waiting = false;
        if self.edge_hidden == hidden && self.edge_motion.is_some() {
            return;
        }
        let now = Instant::now();
        let (position, velocity, _) = self
            .edge_motion
            .map_or((self.edge_progress, 0., true), |motion| motion.sample(now));
        self.edge_progress = position;
        self.edge_hidden = hidden;
        self.edge_motion = Some(diascia::docking::Motion::new(
            position,
            velocity,
            hidden,
            now,
            platform::animations_enabled(),
        ));
        // Keep the same z-order throughout the slide; restore the user's level
        // only once the reveal has settled.
        platform::topmost(self.main.window(), true);
    }
    fn tick_window(&mut self, advance_motion: bool) {
        let now = Instant::now();
        // This guard has to stay first: it is what releases a drag when the window hides or
        // protection escalates, and a stranded drag would block save_window and pin
        // auto_hide_guard's interaction flag forever.
        if !self.main.window().is_visible() || self.policy.protection() != Protection::Normal {
            self.drag = None;
            self.edge_motion = None;
            self.focus_hide.reset();
            return;
        }
        // A launcher drag turns every WM_MOUSEMOVE into a Moved event, so this path runs far
        // more often than the interaction timer. Only the button state matters until it ends;
        // the activation styling catches up on the first tick afterwards.
        if self.drag.is_some() {
            if !platform::mouse_down() {
                self.drag = None;
                if self.settings.edge_hide
                    && let Some((r, work)) = platform::geometry(self.main.window())
                {
                    self.dock = diascia::docking::dock(
                        r,
                        work,
                        (14. * self.main.window().scale_factor()) as i32,
                    );
                    self.dock_work = Some(work);
                    if let Some((_, r)) = self.dock {
                        self.main
                            .window()
                            .set_position(slint::PhysicalPosition::new(r.x, r.y));
                    }
                }
                self.edge_since = now;
            }
            return;
        }
        self.main
            .set_window_active(platform::window_active(self.main.window()));
        if let Some(ui) = self.settings_ui.get() {
            ui.set_window_active(platform::window_active(ui.window()));
        }
        if let Some(editor) = &self.editor {
            editor
                .window
                .set_window_active(platform::window_active(editor.window.window()));
        }
        if self.main.get_search_open() != self.search_was_open {
            if self.main.get_search_open() {
                self.main.set_selected_id(0);
            } else {
                self.main.invoke_restore_grid_focus();
            }
            self.search_was_open = self.main.get_search_open();
            self.render_search();
        }
        let cursor = platform::cursor();
        if self.edge_progress == 0.
            && self.edge_motion.is_none()
            && let Some((_, old)) = self.dock
            && let Some((actual, work)) = platform::geometry(self.main.window())
            && (actual.x != old.x || actual.y != old.y)
        {
            self.dock = diascia::docking::dock(actual, work, 0);
            self.dock_work = Some(work);
            self.edge_since = now;
        }
        if let Some((edge, old)) = self.dock {
            let size = self.main.window().size();
            if let Some(work) =
                platform::work_area_at((old.x + old.width / 2, old.y + old.height / 2))
                && (Some(work) != self.dock_work
                    || size.width as i32 != old.width
                    || size.height as i32 != old.height)
            {
                let waiting = self.protected_edge_waiting;
                if !waiting {
                    self.reveal_edge();
                }
                let mut rect = windowing::Rect {
                    width: size.width as i32,
                    height: size.height as i32,
                    ..old
                };
                rect.x = rect
                    .x
                    .clamp(work.x, work.x + (work.width - rect.width).max(0));
                rect.y = rect
                    .y
                    .clamp(work.y, work.y + (work.height - rect.height).max(0));
                match edge {
                    diascia::docking::Edge::Left => rect.x = work.x,
                    diascia::docking::Edge::Right => rect.x = work.x + work.width - rect.width,
                    diascia::docking::Edge::Top => rect.y = work.y,
                }
                self.dock = Some((edge, rect));
                self.dock_work = Some(work);
                if waiting {
                    let strip = if self.settings.edge_strip_visible {
                        self.settings.edge_strip_width_px as i32
                    } else {
                        0
                    };
                    platform::move_clipped(
                        self.main.window(),
                        diascia::docking::hidden_position(edge, rect, strip),
                        work,
                    );
                } else {
                    self.main
                        .window()
                        .set_position(slint::PhysicalPosition::new(rect.x, rect.y));
                    platform::decorate(self.main.window());
                }
            }
        }
        let paused = (!self.protected_edge_waiting && self.auto_hide_guard().paused())
            // Ordered so the collapsed state (edge_progress == 1.) short-circuits before the
            // GetAsyncKeyState and GetWindowRect round-trips, which it can never use.
            || (self.edge_progress == 0.
                && platform::mouse_down()
                && platform::geometry(self.main.window())
                    .is_some_and(|(rect, _)| diascia::docking::contains(rect, cursor)));
        if paused {
            if self.edge_progress > 0. || self.edge_motion.is_some() {
                self.animate_edge(false);
            } else {
                self.edge_since = now;
                return;
            }
        }
        if let Some((edge, r)) = self.dock {
            self.focus_hide.reset();
            platform::set_opacity(self.main.window(), 1.);
            if !self.settings.edge_hide {
                self.reveal_edge();
                self.dock = None;
                platform::decorate(self.main.window());
                return;
            }
            let strip = if self.settings.edge_strip_visible {
                self.settings.edge_strip_width_px as i32
            } else {
                0
            };
            let band = diascia::docking::reveal_band(edge, r, strip);
            if paused {
                self.edge_since = now;
            } else if self.edge_hidden {
                if diascia::docking::contains(band, cursor)
                    || (self.edge_motion.is_some()
                        && platform::geometry(self.main.window()).is_some_and(|(actual, _)| {
                            diascia::docking::contains(actual, cursor)
                                && self
                                    .dock_work
                                    .is_some_and(|work| diascia::docking::contains(work, cursor))
                        }))
                {
                    if self.edge_since.elapsed() >= Duration::from_millis(40)
                        && self.allowed(ShowSource::Hotkey)
                    {
                        self.animate_edge(false);
                        self.edge_since = now;
                    }
                } else {
                    self.edge_since = now;
                }
            } else if diascia::docking::contains(r, cursor) {
                self.edge_since = now;
            } else if self.edge_motion.is_some() {
                // A recall must finish before starting the leave deadline.
                self.edge_since = now;
            } else if self.edge_since.elapsed() >= Duration::from_millis(150) {
                self.animate_edge(true);
                self.edge_since = now;
            }
            if advance_motion && let Some(motion) = self.edge_motion {
                if let Some(previous) = self.motion_frame.replace(now) {
                    diascia::metrics::record(
                        "animation_frame_ms",
                        now.duration_since(previous).as_secs_f64() * 1000.,
                    );
                }
                let (value, done) = motion.value(now);
                self.edge_progress = value;
                let end = diascia::docking::hidden_position(edge, r, strip);
                let x = (r.x as f32 + (end.0 - r.x) as f32 * value).round() as i32;
                let y = (r.y as f32 + (end.1 - r.y) as f32 * value).round() as i32;
                if let Some(work) = self.dock_work {
                    platform::move_clipped(self.main.window(), (x, y), work);
                }
                if done {
                    self.motion_frame = None;
                    self.edge_motion = None;
                    if !self.edge_hidden {
                        platform::finish_edge_motion(self.main.window());
                        platform::topmost(self.main.window(), self.wants_topmost());
                        windowing::invalidate(self.main.window());
                    }
                }
            }
        } else {
            let opacity = self.focus_hide.opacity(
                self.settings.hide_on_focus_loss,
                platform::app_has_focus(),
                platform::animations_enabled(),
                now,
            );
            platform::set_opacity(self.main.window(), opacity);
            if opacity == 0. && self.settings.hide_on_focus_loss {
                self.hide_all();
                self.focus_hide.reset();
            }
        }
    }
    /// Cursor distance from the reveal band beyond which the collapsed poll drops to half rate.
    const APPROACH_MARGIN_PX: i32 = 96;
    /// A docked launcher spends almost all of its life collapsed, so the interaction timer lands
    /// here on nearly every tick. This does only what that state needs; everything else
    /// `tick_window` checks is covered by an event instead — focus changes arrive as winit Focused
    /// (native event 8), an external move as Moved (event 10), and a display change moves the
    /// window and so also reaches event 10. A reveal runs on the frame clock, whose first frame is
    /// a full tick, so the dock rectangle is re-clamped before the window reaches the screen.
    /// Returns false when this is not the collapsed steady state and a full tick is required.
    fn collapsed_probe(&mut self) -> bool {
        self.collapsed_far = false;
        let (Some((edge, rect)), true) = (self.dock, self.edge_hidden) else {
            return false;
        };
        if self.edge_motion.is_some()
            || self.drag.is_some()
            || self.protected_edge_waiting
            || !self.settings.edge_hide
            || self.main.get_search_open()
            || !self.main.window().is_visible()
            || self.policy.protection() != Protection::Normal
        {
            return false;
        }
        let now = Instant::now();
        self.focus_hide.reset();
        platform::set_opacity(self.main.window(), 1.);
        if self.auto_hide_guard().paused() {
            self.edge_since = now;
            return true;
        }
        let strip = if self.settings.edge_strip_visible {
            self.settings.edge_strip_width_px as i32
        } else {
            0
        };
        let cursor = platform::cursor();
        if diascia::docking::contains(diascia::docking::reveal_band(edge, rect, strip), cursor) {
            if self.edge_since.elapsed() >= Duration::from_millis(40)
                && self.allowed(ShowSource::Hotkey)
            {
                self.animate_edge(false);
                self.edge_since = now;
            }
        } else {
            self.edge_since = now;
            let work = self.dock_work.unwrap_or(rect);
            let approach =
                diascia::docking::approach_band(edge, work, strip, Self::APPROACH_MARGIN_PX);
            self.collapsed_far = !diascia::docking::contains(approach, cursor);
        }
        true
    }

    fn convert_editor_path(&mut self, relative: bool) {
        let Some(entry) = self.editor_draft() else {
            return;
        };
        match diascia::paths::convert_target(&entry.target, &self.data_dir, relative) {
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
    fn extract_editor_icon(&mut self) {
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
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let icon = platform::icon_for(&target, &base);
            let value = serde_json::to_string(&(epoch, requested_target, icon)).unwrap();
            let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_icon_ready(value.into()));
        });
    }
    /// Fill the icons of entries that have none. Every lookup blocks — a Shell call or a favicon
    /// fetch — so the work goes to a background thread and lands in batches.
    fn fill_icons(&self, wanted: Vec<(i64, Target)>) {
        if wanted.is_empty() {
            return;
        }
        let base = self.data_dir.clone();
        let epoch = self.data_epoch;
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let mut batch = Vec::new();
            for (id, target) in wanted {
                let icon = platform::icon_for(&target, &base);
                if !icon.is_empty() {
                    batch.push((id, target.duplicate_key(), icon));
                }
                if batch.len() >= 16 {
                    let value =
                        serde_json::to_string(&(epoch, std::mem::take(&mut batch))).unwrap();
                    if weak
                        .upgrade_in_event_loop(move |ui| ui.invoke_icons_ready(value.into()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
            if !batch.is_empty() {
                let value = serde_json::to_string(&(epoch, batch)).unwrap();
                let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_icons_ready(value.into()));
            }
        });
    }
    fn status(&self, text: impl ToString) {
        let text = text.to_string();
        if self.policy.protection() != Protection::Normal {
            self.notices.borrow_mut().push(text);
            return;
        }
        self.main.set_status(text.into());
    }
    fn refresh(&mut self) -> Result<()> {
        if let Some(initial) = self.initial.take() {
            self.collections = initial.collections;
            self.entries = initial.entries;
        } else {
            self.store.ensure_categories()?;
            self.collections = self.store.collections()?;
            self.entries = self.store.entries()?;
        }
        self.search_index.sync(&self.entries);
        diascia::metrics::record("entry_count", self.entries.len() as f64);
        diascia::metrics::record(
            "entry_blob_bytes",
            self.entries.iter().map(|e| e.icon.len()).sum::<usize>() as f64,
        );
        let ids: std::collections::HashSet<_> = self.entries.iter().map(|e| e.id).collect();
        self.icon_cache.retain(&ids);
        self.icon_index = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, e)| {
                (
                    e.id,
                    diascia::icons::IconKey {
                        fingerprint: diascia::icons::fingerprint(&e.icon),
                        index,
                    },
                )
            })
            .collect();
        self.eligible = None;
        self.collection_names = self
            .collections
            .iter()
            .map(|c| {
                let name = match c
                    .parent_id
                    .and_then(|id| self.collections.iter().find(|p| p.id == id))
                {
                    Some(p) => format!("{} / {}", p.name, c.name).into(),
                    None => c.name.clone().into(),
                };
                (c.id, name)
            })
            .collect();
        self.rebuild_navigation()
    }
    fn rebuild_navigation(&mut self) -> Result<()> {
        if !self
            .collections
            .iter()
            .any(|c| c.id == self.group && c.parent_id.is_none())
        {
            self.group = self
                .collections
                .iter()
                .find(|c| c.parent_id.is_none())
                .unwrap()
                .id;
        }
        if !self
            .collections
            .iter()
            .any(|c| c.id == self.collection && c.parent_id == Some(self.group))
        {
            self.collection = self
                .collections
                .iter()
                .find(|c| c.parent_id == Some(self.group))
                .unwrap()
                .id;
        }
        let groups: Vec<_> = self
            .collections
            .iter()
            .filter(|c| c.parent_id.is_none())
            .map(|c| NavItem {
                id: c.id as i32,
                name: c.name.clone().into(),
            })
            .collect();
        self.main.set_groups(Rc::new(VecModel::from(groups)).into());
        self.main.set_destinations(
            Rc::new(VecModel::from(
                self.collections
                    .iter()
                    .filter(|c| c.parent_id.is_some())
                    .map(|c| NavItem {
                        id: c.id as i32,
                        name: self.collection_name(c.id),
                    })
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        self.main.set_group_id(self.group as i32);
        let categories: Vec<_> = self
            .collections
            .iter()
            .filter(|c| c.parent_id == Some(self.group))
            .map(|c| NavItem {
                id: c.id as i32,
                name: c.name.clone().into(),
            })
            .collect();
        self.main
            .set_categories(Rc::new(VecModel::from(categories)).into());
        self.main.set_category_id(self.collection as i32);
        self.render_items();
        self.restore_view();
        Ok(())
    }
    fn collection_name(&self, id: i64) -> SharedString {
        self.collection_names.get(&id).cloned().unwrap_or_default()
    }
    fn render_items(&mut self) {
        let items: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.collection_id == self.collection)
            .map(|e| self.display_item(e, false))
            .collect();
        let signature = item_signatures(&items);
        if signature != self.items_signature {
            self.items_signature = signature;
            self.grid_ids = items.iter().map(|i| i64::from(i.id)).collect();
            self.collection_ids = self.grid_ids.iter().copied().collect();
            self.main.set_items(Rc::new(VecModel::from(items)).into());
            self.visible_icons.clear();
            self.icon_bands.clear();
        }
        self.render_search();
        self.update_visible_icons();
        self.main.window().request_redraw();
    }
    fn display_item(&self, e: &Entry, search: bool) -> Item {
        Item {
            drag_icon: if self.visible_icons.contains(&e.id) {
                self.icon_cache.get(e.id).1
            } else {
                Default::default()
            },
            icon: if self.visible_icons.contains(&e.id) {
                self.icon_cache.get(e.id).0
            } else {
                Default::default()
            },
            checked: self.browsing.selected.contains(&e.id),
            tooltip_path: diascia::paths::display_target(&e.target, &self.data_dir).into(),
            tooltip_notes: e.notes.clone().into(),
            id: e.id as i32,
            name: e.name.clone().into(),
            detail: if search {
                self.collection_name(e.collection_id)
            } else {
                e.target.value().into()
            },
            kind: e.target.kind_index(),
        }
    }
    fn render_search(&mut self) {
        let scope = self.main.get_search_scope();
        let scope_key = (scope, self.group, self.collection);
        if self
            .eligible
            .as_ref()
            .is_none_or(|(key, _)| *key != scope_key)
        {
            let ids: std::collections::HashSet<_> = self
                .entries
                .iter()
                .filter(|e| {
                    diascia::session::SearchScope::from(scope).includes(
                        e.collection_id,
                        &self.collections,
                        self.group,
                        self.collection,
                    )
                })
                .map(|e| e.id)
                .collect();
            self.eligible = Some((scope_key, ids));
        }
        let matches: Vec<_> = {
            let (_, eligible) = self.eligible.as_ref().expect("scope just resolved");
            self.search_index
                .query(&self.main.get_query(), &self.entries)
                .into_iter()
                .filter(|id| eligible.contains(id))
                .collect()
        };
        let results: Vec<_> = matches
            .iter()
            .filter_map(|id| {
                self.icon_index
                    .get(id)
                    .and_then(|key| self.entries.get(key.index))
                    .map(|e| self.display_item(e, true))
            })
            .collect();
        if self.main.get_search_open()
            && !results.iter().any(|i| i.id == self.main.get_selected_id())
        {
            self.main
                .set_selected_id(results.first().map_or(0, |i| i.id));
        }
        let signature = item_signatures(&results);
        if signature != self.results_signature {
            self.results_signature = signature;
            self.result_ids = results.iter().map(|i| i64::from(i.id)).collect();
            self.main
                .set_results(Rc::new(VecModel::from(results)).into());
            self.visible_icons.clear();
            self.icon_bands.clear();
        }
    }
    fn evaluate(&mut self) {
        let started = Instant::now();
        let mut signals = platform::collect_signals(&self.settings, &mut self.protection_cache);
        signals.auxiliary_open = self.auto_hide_guard().auxiliary_open();
        signals.bypass = self.policy.bypass();
        let hold = signals.hold_release();
        self.apply_protection(decide(signals), Instant::now(), hold);
        self.last_evaluated = started;
        diascia::metrics::record(
            "evaluate_us",
            started.elapsed().as_micros().min(u16::MAX as u128) as f64,
        );
    }
    /// Win-event and display broadcasts are only hints that something may have changed, and a
    /// fullscreen video can deliver them far faster than a shell query costs.
    fn evaluate_suggested(&mut self) {
        if self.last_evaluated.elapsed() < Duration::from_millis(120) {
            diascia::metrics::record("evaluate_dropped", 1.);
            return;
        }
        self.evaluate();
    }
    fn apply_protection(&mut self, signal: Protection, now: Instant, hold: bool) {
        let previous = self.policy.protection();
        let changed = self.policy.update(signal, now, hold);
        if changed {
            diascia::diagnostics::protection(
                "transition",
                "",
                &format!("{:?}", self.policy.protection()),
                &format!(
                    "from {previous:?} hold={hold} bypass={}",
                    self.policy.bypass()
                ),
            );
        }
        self.native_epoch
            .store(self.policy.ticket(), Ordering::SeqCst);
        self.hotkey.set_enabled(
            self.policy.hotkey_enabled()
                && !self
                    .settings_ui
                    .get()
                    .is_some_and(|ui| ui.get_recording_hotkey()),
        );
        if changed && self.policy.protection() != Protection::Normal {
            if previous == Protection::Normal {
                self.save_window(true);
                self.protection_resume.enter(
                    self.main.window().is_visible() && self.dock.is_some(),
                    self.settings.edge_hide,
                    self.main.get_pinned(),
                    &self.saved_window,
                );
            }
            self.hide_windows();
            self.quitting = false;
        } else if changed
            && previous != Protection::Normal
            && self.policy.protection() == Protection::Normal
        {
            // Strict -> Fullscreen is a de-escalation but still not a moment to show anything.
            self.restore_protected_edge();
            let deferred = self.notices.borrow_mut().take();
            if !deferred.is_empty() {
                self.main.set_status(deferred.into());
            }
        }
    }
    fn allowed(&mut self, source: ShowSource) -> bool {
        let ticket = self.policy.ticket();
        self.evaluate();
        let allowed = self.policy.allows(source, ticket);
        if !allowed {
            // Release builds discard stderr, so the block reason has to reach the log to be
            // diagnosable at all.
            diascia::diagnostics::protection(
                "blocked",
                &format!("{source:?}"),
                &format!("{:?}", self.policy.protection()),
                &format!("ticket={ticket} current={}", self.policy.ticket()),
            );
        }
        allowed
    }
    fn hide_all(&mut self) {
        self.protected_edge_waiting = false;
        self.protection_resume.cancel();
        self.hide_windows();
    }
    fn hide_windows(&mut self) {
        self.save_window(true);
        self.interaction_timer.stop();
        self.frame_clock.set_active(false);
        windowing::close_popups(self.main.window());
        if let Some(ui) = self.settings_ui.get() {
            windowing::close_popups(ui.window());
        }
        platform::set_opacity(self.main.window(), 1.);
        platform::topmost(self.main.window(), false);
        let _ = self.main.hide();
        if let Some(ui) = self.settings_ui.get() {
            let _ = ui.hide();
        }
        if let Some(editor) = &self.editor {
            windowing::close_popups(editor.window.window());
            let _ = editor.window.hide();
        }
        // Keep exactly the band the first revealed frame paints, drop the LRU tail the user
        // scrolled past. Nothing re-decodes on the way back in, so the reveal stays one frame.
        // A hide right after a search toggle flushes everything instead: render_search clears
        // visible_icons without repopulating it.
        self.icon_cache.retain(&self.visible_icons);
        self.main.invoke_close_popups();
    }
    fn show_main(&mut self, source: ShowSource) {
        if !self.allowed(source) {
            return;
        }
        let visible_edge = self.main.window().is_visible()
            && self.dock.is_some()
            && (self.edge_progress > 0. || self.edge_motion.is_some());
        if !self.main.window().is_visible()
            && let Err(e) = windowing::show(self.main.window())
        {
            self.status(e);
            return;
        }
        self.initialize_native_state();
        if visible_edge && !self.auto_hide_guard().auxiliary_open() {
            self.animate_edge(false);
            self.edge_since = Instant::now();
        } else {
            self.user_summon_edge();
        }
        self.schedule_interaction();
        diascia::metrics::record("show_requested", source as u8 as f64);
        platform::launcher_window(self.main.window());
        platform::topmost(
            self.main.window(),
            self.edge_motion.is_some() || self.wants_topmost(),
        );
        platform::focus(self.main.window());
        self.focus_hide.reset();
        if self.settings_open {
            let _ = windowing::show_child(self.settings_ui.window(), self.main.window());
        }
        if let Some(editor) = &self.editor {
            let _ = windowing::show_child(editor.window.window(), self.main.window());
        }
    }
    fn show_settings(&mut self) {
        if !self.allowed(ShowSource::Settings) {
            return;
        }
        self.user_summon_edge();
        if self.settings_ui.get().is_none() {
            match create_settings(
                &self.weak_self.upgrade().expect("live application"),
                &self.data_dir,
            ) {
                Ok(ui) => {
                    let _ = self.settings_ui.0.set(ui);
                    self.sync_settings_ui();
                }
                Err(e) => {
                    self.status(e);
                    return;
                }
            }
        }
        self.show_main(ShowSource::Settings);
        self.settings_open = true;
        if let Err(error) = windowing::show_child(self.settings_ui.window(), self.main.window()) {
            self.settings_open = false;
            self.status(error);
        }
    }
    fn close_settings(&mut self) {
        self.settings_ui.set_recording_hotkey(false);
        self.sync_settings_ui();
        self.focus_hide.reset();
        self.edge_since = Instant::now();
        if let Err(error) = windowing::close_child(self.settings_ui.window(), self.main.window()) {
            self.status(error);
            return;
        }
        self.settings_open = false;
        self.evaluate();
    }
    fn sync_main_settings(&self) {
        self.main.set_double_click(self.settings.double_click);
        self.main.set_icon_size(i32::from(self.settings.icon_size));
        self.main.set_list_view(self.settings.list_view);
        self.main.set_hover_switch(self.settings.hover_switch);
        self.main
            .set_hover_delay(self.settings.hover_delay_ms as i32);
    }
    fn sync_settings_ui(&self) {
        self.sync_main_settings();
        if self.settings_ui.get().is_none() {
            return;
        }
        self.settings_ui
            .set_hotkey_text(self.settings.hotkey.clone().into());
        self.settings_ui.set_protected_error("".into());
        self.settings_ui.set_autostart(self.settings.autostart);
        self.settings_ui
            .set_double_click(self.settings.double_click);
        self.settings_ui.set_edge_hide(self.settings.edge_hide);
        self.settings_ui
            .set_keep_topmost(self.saved_window.always_on_top);
        self.settings_ui
            .set_edge_strip_visible(self.settings.edge_strip_visible);
        self.settings_ui
            .set_edge_strip_width(self.settings.edge_strip_width_px as i32);
        self.settings_ui
            .set_hide_on_focus_loss(self.settings.hide_on_focus_loss);
        self.settings_ui
            .set_hover_switch(self.settings.hover_switch);
        self.settings_ui
            .set_hover_delay(self.settings.hover_delay_ms as i32);
        self.settings_ui
            .set_relative_paths(self.settings.relative_paths);
        self.sync_main_settings();
        self.settings_ui
            .set_hide_after_launch(self.settings.hide_after_launch);
        self.settings_ui
            .set_protection_log(self.settings.protection_log);
        self.settings_ui.set_protection_bypass(self.policy.bypass());
        self.settings_ui
            .set_fullscreen_protection(self.settings.fullscreen_protection);
        self.settings_ui
            .set_protected_apps(strings(self.settings.protected_apps.clone()));
    }
    fn settings_value(&self) -> Settings {
        let mut draft = self.settings.clone();
        draft.hotkey = self.settings_ui.get_hotkey_text().to_string();
        draft.autostart = self.settings_ui.get_autostart();
        draft.double_click = self.settings_ui.get_double_click();
        draft.edge_hide = self.settings_ui.get_edge_hide();
        draft.edge_strip_visible = self.settings_ui.get_edge_strip_visible();
        draft.edge_strip_width_px = self.settings_ui.get_edge_strip_width() as u32;
        draft.hide_on_focus_loss = self.settings_ui.get_hide_on_focus_loss();
        draft.hover_switch = self.settings_ui.get_hover_switch();
        draft.hover_delay_ms = self.settings_ui.get_hover_delay() as u32;
        draft.relative_paths = self.settings_ui.get_relative_paths();
        draft.hide_after_launch = self.settings_ui.get_hide_after_launch();
        draft.fullscreen_protection = self.settings_ui.get_fullscreen_protection();
        draft.protection_log = self.settings_ui.get_protection_log();
        draft.protected_apps = self
            .settings_ui
            .get_protected_apps()
            .iter()
            .map(|path| path.trim().trim_matches('"').to_string())
            .filter(|path| !path.is_empty())
            .collect();
        draft
    }
    fn apply_settings(&mut self, field: i32) {
        if field == 3 {
            self.save_window(true);
            let mut state = self.saved_window.clone();
            state.always_on_top = self.settings_ui.get_keep_topmost();
            match self.store.save_window_state(&state) {
                Ok(()) => {
                    self.saved_window = state;
                    self.window_candidate = None;
                    platform::topmost(
                        self.main.window(),
                        self.wants_topmost()
                            || (self.policy.protection() == Protection::Normal && self.edge_hidden),
                    );
                    self.settings_ui.set_status("".into());
                }
                Err(e) => {
                    self.settings_ui
                        .set_keep_topmost(self.saved_window.always_on_top);
                    self.settings_ui.set_status(e.to_string().into());
                }
            }
            return;
        }
        let draft = self.settings_value();
        if let Err(e) = diascia::storage::validate_settings(&draft) {
            if field == 1 {
                // Roll the list back so an invalid entry cannot leak into the
                // next settings_value read from another field's apply.
                self.settings_ui
                    .set_protected_apps(strings(self.settings.protected_apps.clone()));
                self.settings_ui.set_protected_error(e.to_string().into());
            } else {
                self.settings_ui.set_status(e.to_string().into());
            }
            return;
        }
        if field == 1 {
            self.settings_ui.set_protected_error("".into());
        }
        if draft == self.settings {
            self.settings_ui.set_status("".into());
            return;
        }
        if draft.hotkey != self.settings.hotkey
            && let Err(error) = self
                .hotkey
                .configure(&draft.hotkey, self.policy.hotkey_enabled())
        {
            self.settings_ui
                .set_hotkey_text(self.settings.hotkey.clone().into());
            self.settings_ui.set_hotkey_error(error.into());
            return;
        }
        if draft.autostart != self.settings.autostart
            && let Err(e) = platform::set_autostart(draft.autostart, &self.data_dir)
        {
            self.sync_settings_ui();
            self.settings_ui.set_status(e.into());
            return;
        }
        match self.store.save_settings(&draft) {
            Ok(()) => {
                self.user_summon_edge();
                if !draft.edge_hide {
                    self.protection_resume.cancel();
                    self.dock = None;
                }
                self.settings = draft;
                diascia::diagnostics::set_protection_log(self.settings.protection_log);
                self.settings_ui.set_hotkey_error("".into());
                self.sync_main_settings();
                self.settings_ui.set_status("".into());
                self.evaluate();
            }
            Err(e) => {
                if draft.hotkey != self.settings.hotkey {
                    let _ = self
                        .hotkey
                        .configure(&self.settings.hotkey, self.policy.hotkey_enabled());
                }
                if draft.autostart != self.settings.autostart {
                    let _ = platform::set_autostart(self.settings.autostart, &self.data_dir);
                }
                self.sync_settings_ui();
                self.settings_ui.set_status(e.to_string().into());
            }
        }
    }
    fn add_protected_app(&mut self, path: String) {
        let normalize = |path: &str| -> String {
            path.trim()
                .trim_matches('"')
                .replace('/', "\\")
                .to_lowercase()
        };
        let path = path.trim().trim_matches('"').to_string();
        if path.is_empty() {
            return;
        }
        if self
            .settings
            .protected_apps
            .iter()
            .any(|existing| normalize(existing) == normalize(&path))
        {
            self.settings_ui
                .set_protected_error("该程序已在保护列表中".into());
            return;
        }
        if self.settings.protected_apps.len() >= 100 {
            self.settings_ui
                .set_protected_error("严格保护程序最多 100 项".into());
            return;
        }
        let mut list = self.settings.protected_apps.clone();
        list.push(path);
        self.settings_ui.set_protected_apps(strings(list));
        self.apply_settings(1);
    }
    fn remove_protected(&mut self, index: i32) {
        if index < 0 {
            return;
        }
        let mut list = self.settings.protected_apps.clone();
        if (index as usize) >= list.len() {
            return;
        }
        list.remove(index as usize);
        self.settings_ui.set_protected_apps(strings(list));
        self.apply_settings(1);
    }
    fn launch_elevated(&mut self, id: i64) {
        self.launch_with_elevation(id, true);
    }
    fn launch(&mut self, id: i64) {
        self.launch_with_elevation(id, false);
    }
    fn launch_with_elevation(&mut self, id: i64, elevated: bool) {
        if self.main.get_busy() || !self.allowed(ShowSource::Hotkey) {
            return;
        }
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        let mut target = diascia::paths::convert_target(&entry.target, &self.data_dir, false)
            .unwrap_or_else(|_| entry.target.clone());
        if elevated {
            target = match target {
                Target::Program {
                    path,
                    arguments,
                    working_dir,
                    show,
                    ..
                } => Target::Program {
                    path,
                    arguments,
                    working_dir,
                    elevated: true,
                    show,
                },
                Target::Shortcut { path, show } => Target::Program {
                    path,
                    arguments: String::new(),
                    working_dir: String::new(),
                    elevated: true,
                    show,
                },
                _ => {
                    self.status("该类型不支持管理员打开");
                    return;
                }
            };
        }
        self.main.set_busy(true);
        self.main.set_status("".into());
        let weak = self.main.as_weak();
        let settings = self.settings.clone();
        std::thread::spawn(move || {
            let failure = if platform::protection(&settings) == Protection::Normal {
                platform::launch(&target).err().map(|error| Failure {
                    entry: id,
                    message: error.message(),
                    relocatable: error.relocatable(),
                })
            } else {
                Some(Failure {
                    entry: id,
                    message: "当前处于全屏或游戏保护，已取消启动".into(),
                    relocatable: false,
                })
            };
            // An empty payload still means success, matching the previous callback contract.
            let payload = failure
                .map(|failure| serde_json::to_string(&failure).unwrap_or_default())
                .unwrap_or_default();
            let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_launch_finished(payload.into()));
        });
    }
    /// Arms the in-window relocate panel. Returns false when the target has no path to
    /// re-point, so callers can fall back to a plain status message.
    fn show_relocate(&mut self, id: i64, message: &str) -> bool {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return false;
        };
        if !entry.target.relocatable() {
            return false;
        }
        self.relocate_pending = id;
        self.main.set_relocate_name(entry.name.clone().into());
        self.main.set_relocate_target(entry.target.value().into());
        self.main.set_relocate_message(message.into());
        self.main.set_relocate_open(true);
        true
    }
    fn begin_relocate(&mut self) {
        if self.relocate_pending == 0 {
            return;
        }
        self.dialog(7);
    }
    fn apply_relocate(&mut self, value: String) -> Result<()> {
        let id = self.relocate_pending;
        let Some(original) = self.entries.iter().find(|e| e.id == id).cloned() else {
            self.relocate_pending = 0;
            return Ok(());
        };
        let mut updated = original.clone();
        updated.target = original.target.with_path(&value)?;
        // The embedded icon was extracted from the target this entry used to point at.
        updated.icon = Vec::new();
        self.store.update_targets_checked(&[updated], &[original])?;
        self.relocate_pending = 0;
        self.main.set_relocate_open(false);
        self.report_refresh();
        Ok(())
    }
    fn editor_draft(&self) -> Option<Entry> {
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
    fn editor_dirty(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|e| self.editor_draft().as_ref() != Some(&e.original))
    }
    fn save_editor(&mut self) {
        if self.icon_pending {
            if let Some(e) = &self.editor {
                e.window.set_error("正在读取图标，请稍候保存".into());
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
            && let Ok(target) = diascia::paths::convert_target(&entry.target, &self.data_dir, true)
        {
            entry.target = target;
        }
        if let Err(e) = entry.validate() {
            if let Some(session) = &self.editor {
                session.window.set_error(e.to_string().into());
            }
            return;
        }
        if self
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
            Ok(_) => {
                self.drop_editor();
                self.report_refresh();
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
    fn cancel_editor(&mut self) {
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
    fn drop_editor(&mut self) {
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
    fn request_quit(&mut self) {
        self.protection_resume.cancel();
        self.save_window(true);
        diascia::metrics::flush();
        if self.importing || !self.pending_files.is_empty() {
            self.status("正在收录文件，请完成后再退出");
            return;
        }
        if self.file_dialog_open {
            self.status("请先关闭文件选择窗口");
            return;
        }
        self.quitting = true;
        if self.editor_dirty() {
            if self.allowed(ShowSource::Editor) {
                let editor = self.editor.as_ref().unwrap();
                editor.window.set_discard_confirm(true);
                let _ = windowing::show_child(editor.window.window(), self.main.window());
            }
            return;
        }
        let _ = slint::quit_event_loop();
    }
    fn report_refresh(&mut self) {
        if self.main.get_category_id() as i64 == self.collection {
            self.remember_view();
        }
        if let Err(e) = self.refresh() {
            self.status(e);
        }
    }
    fn dialog(&mut self, kind: i32) {
        if self.file_dialog_open || !self.allowed(ShowSource::Settings) {
            return;
        }
        let folder = match kind {
            0 => self
                .editor
                .as_ref()
                .is_some_and(|s| s.window.get_kind() == 2),
            7 => self.entries.iter().any(|e| {
                e.id == self.relocate_pending && matches!(e.target, Target::Folder { .. })
            }),
            _ => false,
        };
        self.file_dialog_open = true;
        self.user_summon_edge();
        self.dialog_collection = self.collection;
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new();
            if kind == 5 {
                let paths = dialog.pick_files().unwrap_or_default();
                let value = serde_json::to_string(&paths).unwrap_or_default();
                let _ = weak
                    .upgrade_in_event_loop(move |ui| ui.invoke_file_selected(kind, value.into()));
                return;
            }
            let path = match kind {
                0 if folder => dialog.pick_folder(),
                0 => dialog.pick_file(),
                6 => dialog
                    .add_filter("图标", &["png", "jpg", "jpeg", "ico"])
                    .pick_file(),
                1 => dialog.add_filter("程序", &["exe"]).pick_file(),
                2 => dialog
                    .add_filter("Diascia 导出", &["json"])
                    .set_file_name("diascia-export.json")
                    .save_file(),
                3 => dialog
                    .add_filter("SQLite 备份", &["db"])
                    .set_file_name("diascia-backup.db")
                    .save_file(),
                7 if folder => dialog.pick_folder(),
                7 => dialog.pick_file(),
                _ => dialog.add_filter("Diascia 导出", &["json"]).pick_file(),
            };
            let value = path
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let _ =
                weak.upgrade_in_event_loop(move |ui| ui.invoke_file_selected(kind, value.into()));
        });
    }
    fn file_selected(&mut self, kind: i32, value: String) -> Result<()> {
        self.file_dialog_open = false;
        self.focus_hide.reset();
        self.edge_since = Instant::now();
        if value.is_empty() {
            return Ok(());
        }
        let path = PathBuf::from(&value);
        match kind {
            6 => {
                let icon = diascia::icons::from_file(&path)?;
                self.icon_epoch += 1;
                self.icon_pending = false;
                if let Some(session) = &mut self.editor {
                    session
                        .window
                        .set_entry_icon(diascia::icons::display(&icon));
                    session.icon = icon;
                }
            }
            5 => {
                let paths: Vec<PathBuf> = serde_json::from_str(&value)?;
                self.queue_files(paths, self.dialog_collection)?;
            }
            0 => {
                if let Some(session) = &self.editor {
                    session.window.set_target(value.into());

                    if session.window.get_entry_name().is_empty() {
                        session.window.set_entry_name(
                            path.file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                                .into(),
                        );
                    }
                    if path.extension().is_some_and(|e| {
                        e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url")
                    }) {
                        session.window.set_kind(4);
                    }
                }
            }
            1 => {
                self.add_protected_app(value);
            }
            2 => {
                use std::io::Write;
                let bytes = serde_json::to_vec_pretty(&self.store.export()?)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                self.settings_ui.set_status("已导出".into());
            }
            3 => {
                self.store.backup(&path)?;
                self.settings_ui.set_status("已备份".into());
            }
            7 => self.apply_relocate(value)?,
            _ => {
                if self.editor.is_some() {
                    return Err(Error::Invalid("请先保存或取消项目编辑".into()));
                }
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(32 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(Error::Invalid("导出文件超过 32 MB".into()));
                }
                let data: Export = serde_json::from_slice(&bytes)?;
                diascia::storage::validate_export(&data)?;
                self.settings_ui.set_status(
                    format!(
                        "将恢复 {} 个分组/分类、{} 个项目",
                        data.collections.len(),
                        data.entries.len()
                    )
                    .into(),
                );
                self.pending_import = Some(data);
                self.settings_ui.set_restore_confirm(true);
            }
        }
        if kind == 0 {
            self.extract_editor_icon();
        }
        Ok(())
    }
    fn queue_files(&mut self, paths: Vec<PathBuf>, collection: i64) -> Result<()> {
        let overflow = self.pending_files.len() + paths.len() > 500;
        // Keep one overflow sentinel item so the worker rejects the entire drop,
        // rather than silently committing its first 500 paths.
        let capacity = 501usize.saturating_sub(self.pending_files.len());
        self.pending_files
            .extend(paths.into_iter().take(capacity).map(|p| (p, collection)));
        self.last_drop = Instant::now();
        if overflow {
            return Err(Error::Invalid("每次最多添加 500 项，请分批拖入".into()));
        }
        Ok(())
    }
    fn process_files(&mut self) {
        if self.importing
            || self.pending_files.is_empty()
            || self.last_drop.elapsed() < Duration::from_millis(120)
        {
            return;
        }
        self.importing = true;
        self.status("正在读取文件…");
        let paths = std::mem::take(&mut self.pending_files);
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let result = diascia::importing::file_entries(paths).map_err(|e| e.to_string());
            let value = serde_json::to_string(&result).expect("serializable import result");
            let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_files_ready(value.into()));
        });
    }
    fn restore(&mut self) -> Result<()> {
        if self.importing || !self.pending_files.is_empty() || self.main.get_busy() {
            return Err(Error::Invalid("请先等待文件收录完成，再恢复数据".into()));
        }
        if self.editor.is_some() {
            return Err(Error::Invalid("请先保存或取消项目编辑".into()));
        }
        let Some(data) = self.pending_import.as_ref() else {
            return Ok(());
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup = self.data_dir.join(format!("before-restore-{stamp}.db"));
        self.store.backup(&backup)?;
        let old_hotkey = self.settings.hotkey.clone();
        self.hotkey
            .configure(&data.settings.hotkey, self.policy.hotkey_enabled())
            .map_err(Error::Invalid)?;
        if let Err(error) = self.store.restore(data) {
            let _ = self
                .hotkey
                .configure(&old_hotkey, self.policy.hotkey_enabled());
            return Err(error);
        }
        self.data_epoch += 1;
        self.clear_batch();
        self.browsing.views.clear();
        self.browsing.group_categories.clear();
        self.settings = self.store.settings()?;
        self.sync_settings_ui();
        self.refresh()?;
        self.pending_import = None;
        self.settings_ui.set_restore_confirm(false);
        self.settings_ui
            .set_status(format!("已恢复。原数据备份：{}", backup.display()).into());
        self.evaluate();
        Ok(())
    }
}

fn open_editor(app: &Rc<RefCell<App>>, id: i64) {
    let mut state = app.borrow_mut();
    if id != 0 && state.main.get_batch_mode() {
        return;
    }
    if state.file_dialog_open {
        state.status("请先关闭文件选择窗口");
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
            session.window.set_error("请先保存或取消当前编辑".into());
        }
        let _ = windowing::show_child(session.window.window(), state.main.window());
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
    ui.set_entry_icon(diascia::icons::display(&entry.icon));
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
    bind!(on_browse, |s: &mut App| s.dialog(0));
    bind!(on_browse_icon, |s: &mut App| s.dialog(6));
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
    let weak = Rc::downgrade(app);
    ui.on_icon_path(move |path| {
        if let Some(app) = weak.upgrade() {
            let mut s = app.borrow_mut();
            if !s.file_dialog_open
                && !s.saving_editor
                && let Err(e) = s.file_selected(6, path.to_string())
                && let Some(session) = &s.editor
            {
                session.window.set_error(e.to_string().into());
            }
        }
    });
    let icon_weak = ui.as_weak();
    let native_weak = state.main.as_weak();
    let generation = state.editor_generation;
    let weak = ui.as_weak();
    install_keys(
        ui.window(),
        move |key| {
            if key == slint::winit_030::winit::keyboard::NamedKey::Escape {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_cancel();
                }
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
                let _ = native_weak.upgrade_in_event_loop(move |ui| {
                    ui.invoke_native_event(12, generation.to_string().into())
                });
            }
            if let slint::winit_030::winit::event::WindowEvent::Focused(active) = event
                && let Some(ui) = icon_weak.upgrade()
            {
                ui.set_window_active(*active);
            }
        },
        |_| {},
        false,
    );
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
    if let Err(e) = windowing::show_child(ui.window(), state.main.window()) {
        state.status(e);
    }
    let weak = state.main.as_weak();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_native_event(12, generation.to_string().into());
        }
    });
    if state
        .editor
        .as_ref()
        .is_some_and(|e| e.icon.is_empty() && !e.window.get_target().is_empty())
    {
        state.extract_editor_icon();
    }
}

/// Debug-only evidence trail for real input-method behaviour, which no automated test here can
/// produce. Release builds have no console, so this is a development affordance only.
fn ime_trace(what: &str, active: bool) {
    if cfg!(debug_assertions) && std::env::var_os("DIASCIA_IME_TRACE").is_some() {
        eprintln!("[ime] {what} composing={active}");
    }
}

fn install_keys(
    window: &slint::Window,
    action: impl Fn(slint::winit_030::winit::keyboard::NamedKey) -> bool + 'static,
    event_action: impl Fn(&slint::winit_030::winit::event::WindowEvent) + 'static,
    compose_sink: impl Fn(bool) + 'static,
    guard_drag: bool,
) {
    use slint::winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::{ElementState, Ime, WindowEvent},
            keyboard::{Key, NamedKey},
        },
    };
    let mut gesture = diascia::gestures::DragGuard::default();
    let mut pointer = (0., 0.);
    let mut full_frame_pending = false;
    let mut composition = diascia::gestures::CompositionGuard::default();
    let mut control = false;
    let mut launcher = platform::LauncherProbe::default();
    window.on_winit_window_event(move |window, event| {
        if guard_drag {
            launcher.ensure(window);
        }
        event_action(event);
        if guard_drag {
            use slint::winit_030::winit::event::MouseButton;
            match event {
                WindowEvent::CursorMoved { position, .. } => {
                    let p = position.to_logical::<f64>(window.scale_factor() as f64);
                    pointer = (p.x, p.y);
                    if gesture.suppress_move(pointer, Instant::now()) { return EventResult::PreventDefault; }
                }
                WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                    if *state == ElementState::Pressed && pointer.1 >= 44. {
                        gesture.press(pointer, Instant::now());
                    } else if *state == ElementState::Released && gesture.release() {
                        window.dispatch_event(slint::platform::WindowEvent::PointerExited);
                    }
                }
                WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. } => { gesture.reset(); }
                _ => {}
            }
        }
        match event {
            WindowEvent::ModifiersChanged(modifiers) => control = modifiers.state().control_key(),
            WindowEvent::Focused(_) => platform::refresh_border(window),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) => {
                platform::refresh_geometry(window);
                // The size is still settling here, so the full repaint waits for this frame.
                full_frame_pending = true;
            }
            WindowEvent::RedrawRequested => {
                windowing::prepare_tooltips(window);
                if full_frame_pending {
                    full_frame_pending = false;
                    windowing::mark_full_frame(window);
                }
            }
            WindowEvent::Ime(value) => {
                use diascia::gestures::ImeSignal;
                let signal = match value {
                    Ime::Preedit(_, cursor) => ImeSignal::Preedit { has_cursor: cursor.is_some() },
                    Ime::Commit(_) => ImeSignal::Commit,
                    Ime::Enabled | Ime::Disabled => ImeSignal::Disabled,
                };
                compose_sink(composition.apply(signal));
                ime_trace("transition", composition.active());
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed =>
            {
                if composition.active() {
                    // Enter belongs to the candidate window while composing; characters must
                    // still reach the focused editor, which is the only commit-text channel.
                    let swallowed = matches!(event.logical_key, Key::Named(NamedKey::Enter));
                    if swallowed {
                        ime_trace("Enter", true);
                        return EventResult::PreventDefault;
                    }
                    return EventResult::Propagate;
                }
                if windowing::has_popup(window) {
                    return EventResult::Propagate;
                }
                if control && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("a")) && action(slint::winit_030::winit::keyboard::NamedKey::Select) { return EventResult::PreventDefault; }
                if control && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("f")) && action(slint::winit_030::winit::keyboard::NamedKey::Find) { return EventResult::PreventDefault; }
                if let Key::Named(key) = event.logical_key
                    && action(key)
                {
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn install_settings_keys(ui: &SettingsWindow) {
    use slint::winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::{ElementState, Ime, WindowEvent},
            keyboard::{Key, NamedKey, PhysicalKey},
        },
    };
    let weak = ui.as_weak();
    let mut modifiers = slint::winit_030::winit::keyboard::ModifiersState::empty();
    let mut composition = diascia::gestures::CompositionGuard::default();
    let mut full_frame_pending = false;
    ui.window().on_winit_window_event(move |window, event| {
        match event {
            WindowEvent::Focused(active) => {
                if let Some(ui) = weak.upgrade() {
                    ui.set_window_active(*active);
                }
            }
            WindowEvent::RedrawRequested => {
                windowing::prepare_tooltips(window);
                if full_frame_pending {
                    full_frame_pending = false;
                    windowing::mark_full_frame(window);
                }
            }
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::ThemeChanged(_) => {
                platform::refresh_geometry(window);
                full_frame_pending = true;
            }
            WindowEvent::ModifiersChanged(value) => modifiers = value.state(),
            WindowEvent::Ime(value) => {
                use diascia::gestures::ImeSignal;
                let signal = match value {
                    Ime::Preedit(_, cursor) => ImeSignal::Preedit {
                        has_cursor: cursor.is_some(),
                    },
                    Ime::Commit(_) => ImeSignal::Commit,
                    Ime::Enabled | Ime::Disabled => ImeSignal::Disabled,
                };
                let active = composition.apply(signal);
                if let Some(ui) = weak.upgrade() {
                    ui.set_composing(active);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(ui) = weak.upgrade()
                    && ui.get_recording_hotkey()
                {
                    // A key pressed while composing belongs to the input method, not to the
                    // shortcut being recorded.
                    if composition.active() {
                        ime_trace("recording key ignored", true);
                        return EventResult::PreventDefault;
                    }
                    if event.logical_key == Key::Named(NamedKey::Escape) {
                        ui.invoke_captured_hotkey("".into());
                        return EventResult::PreventDefault;
                    }
                    let name = match event.physical_key {
                        PhysicalKey::Code(code) => {
                            diascia::hotkey::physical_name(&format!("{code:?}"))
                        }
                        _ => None,
                    };
                    let Some(name) = name else {
                        return EventResult::PreventDefault;
                    };
                    // Query at keydown as well: some layouts consume Alt/Ctrl modifier events.
                    let native_modifiers = platform::key_modifiers();
                    let mut parts = vec![];
                    if modifiers.control_key() || native_modifiers & 2 != 0 {
                        parts.push("Ctrl".to_string());
                    }
                    if modifiers.alt_key() || native_modifiers & 1 != 0 {
                        parts.push("Alt".to_string());
                    }
                    if modifiers.shift_key() || native_modifiers & 4 != 0 {
                        parts.push("Shift".to_string());
                    }
                    if modifiers.super_key() || native_modifiers & 8 != 0 {
                        parts.push("Win".to_string());
                    }
                    parts.push(name);
                    ui.invoke_captured_hotkey(parts.join("+").into());
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut data_dir = None;
    let mut portable = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            data_dir = Some(PathBuf::from(args.next().ok_or("--data-dir 需要目录")?));
        } else if arg == "--portable" {
            portable = true;
        } else {
            return Err(format!("未知参数：{}", arg.to_string_lossy()).into());
        }
    }
    if portable && data_dir.is_some() {
        return Err("--portable 与 --data-dir 不能同时使用".into());
    }
    run_at(match data_dir {
        Some(path) => path,
        None => std::env::current_exe()?
            .parent()
            .ok_or("无法确定程序目录")?
            .to_path_buf(),
    })
}

fn run_at(data_dir: PathBuf) -> std::result::Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&data_dir)?;
    let data_dir = data_dir.canonicalize()?;
    diascia::diagnostics::attach(&data_dir);
    // Release builds have no console, so a worker-thread panic is otherwise silent. Still print
    // to stderr: the test harness relies on the default rendering of a failing panic.
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map_or_else(
            || "<未知位置>".into(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<非字符串 panic>".into());
        eprintln!("panic at {location}: {payload}");
        diascia::diagnostics::error("panic", &format!("{location} {payload}"));
    }));
    let Some(mut instance) = platform::Instance::acquire(&data_dir).map_err(Error::Invalid)? else {
        return Ok(());
    };
    let mut store = Store::open(&data_dir.join("diascia.db"))?;
    let initial = store.take_initial()?;
    let settings = initial.settings.clone();
    diascia::diagnostics::set_protection_log(settings.protection_log);
    let saved_window = store.window_state()?;
    diascia::metrics::record("database_ready", 0.);
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true).with_drag_and_drop(false)
        });
    #[cfg(test)]
    let selector = selector.with_winit_event_loop_builder({
        use slint::winit_030::winit::platform::windows::EventLoopBuilderExtWindows;
        let mut builder = slint::winit_030::winit::event_loop::EventLoop::with_user_event();
        builder.with_any_thread(true);
        builder
    });
    selector.select()?;
    let main = MainWindow::new()?;
    let mut policy = DisplayPolicy::default();
    let signals = platform::collect_signals(&settings, &mut platform::ProtectionCache::default());
    policy.update(decide(signals), Instant::now(), signals.hold_release());
    let native_epoch = Arc::new(AtomicU64::new(policy.ticket()));
    let epoch = native_epoch.clone();
    let weak = main.as_weak();
    let notify: platform::Notify = Arc::new(move |code, text| {
        diascia::metrics::record("native_received", code as f64);
        let text = if matches!(code, 1 | 3 | 4 | 6 | 7) {
            epoch.load(Ordering::SeqCst).to_string()
        } else {
            text
        };
        let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_native_event(code, text.into()));
    });
    let hotkey = platform::Hotkey::new(notify.clone(), policy.hotkey_enabled(), &settings.hotkey)
        .map_err(Error::Invalid)?;
    #[cfg(debug_assertions)]
    eprintln!("hotkey worker ready");
    instance.listen(notify.clone());
    let foreground = platform::ForegroundWatch::new(notify.clone()).map_err(Error::Invalid)?;
    let frame_clock = platform::FrameClock::new(notify.clone()).map_err(Error::Invalid)?;
    let tray = platform::Tray::new(notify).map_err(Error::Invalid)?;
    #[cfg(debug_assertions)]
    eprintln!("tray ready");
    let app = Rc::new(RefCell::new(App {
        protection_resume: Default::default(),
        editor_generation: 0,
        batch_expanded: Default::default(),
        editor_expanded: Default::default(),
        external_drop: None,
        main: main.clone_strong(),
        weak_self: Default::default(),
        interaction_timer: Default::default(),
        frame_clock,
        position_restored: false,
        store,
        data_dir,
        collections: vec![],
        entries: vec![],
        settings,
        policy,
        protection_cache: Default::default(),
        native_epoch,
        hotkey,
        _foreground: foreground,
        tray,
        last_evaluated: Instant::now(),
        notices: Default::default(),
        _instance: instance,
        initial: Some(initial),
        data_epoch: 0,
        group: saved_window.group,
        collection: saved_window.category,
        saved_window,
        window_candidate: None,
        browsing: Default::default(),
        editor: None,
        installed_apps: Vec::new(),
        settings_ui: Deferred::default(),
        settings_open: false,
        drag: None,
        dock: None,
        edge_hidden: false,
        collapsed_far: false,
        protected_edge_waiting: false,
        dock_work: None,
        edge_progress: 0.,
        edge_motion: None,
        motion_frame: None,
        focus_hide: Default::default(),
        edge_since: Instant::now(),
        search_index: Default::default(),
        icon_cache: {
            let weak = main.as_weak();
            diascia::icons::Cache::new(move || {
                let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_native_event(9, "".into()));
            })
        },
        icon_index: Default::default(),
        visible_icons: Default::default(),
        items_signature: Default::default(),
        results_signature: Default::default(),
        grid_ids: Default::default(),
        result_ids: Default::default(),
        icon_bands: Default::default(),
        collection_ids: Default::default(),
        collection_names: Default::default(),
        eligible: None,
        scope_selection: Default::default(),
        conversion_preview: None,
        delete_collection_id: 0,
        relocate_pending: 0,
        search_was_open: false,
        pending_import: None,
        quitting: false,
        file_dialog_open: false,
        dialog_collection: 0,
        pending_files: vec![],
        last_drop: Instant::now(),
        importing: false,
        icon_epoch: 0,
        icon_pending: false,
        saving_editor: false,
    }));
    app.borrow_mut().weak_self = Rc::downgrade(&app);
    macro_rules! bind { ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => { { let weak = Rc::downgrade(&app); $ui.$callback(move |$($arg),*| { if let Some(app) = weak.upgrade() { #[allow(unused_mut)] let mut $s = app.borrow_mut(); $body } }); } }; }
    bind!(main, on_search, |s, _query| {
        let started = Instant::now();
        s.reset_batch_selection();
        s.main.set_selected_id(0);
        s.render_search();
        s.update_visible_icons();
        diascia::metrics::record(
            "search_render_us",
            started.elapsed().as_micros().min(u16::MAX as u128) as f64,
        );
    });
    main.on_make_drag(|id| {
        let mut data = slint::DataTransfer::default();
        data.set_user_data(Rc::new(DraggedEntry(i64::from(id))));
        data
    });
    let weak = Rc::downgrade(&app);
    main.on_can_drop(move |data| {
        weak.upgrade().is_some_and(|app| {
            let s = app.borrow();
            !s.main.get_busy()
                && !s.main.get_batch_mode()
                && s.main.get_query().is_empty()
                && dragged_id(&data).is_some_and(|id| s.collection_ids.contains(&id))
        })
    });
    let weak = Rc::downgrade(&app);
    main.on_drop_before(move |data, before, after| {
        let (Some(app), Some(id)) = (weak.upgrade(), dragged_id(&data)) else {
            return false;
        };
        let mut s = app.borrow_mut();
        if !s.main.get_query().is_empty() || s.main.get_busy() || s.main.get_batch_mode() {
            return false;
        }
        match s.store.move_at(id, i64::from(before), after) {
            Ok(()) => {
                s.main.set_selected_id(id as i32);
                s.report_refresh();
                true
            }
            Err(e) => {
                s.status(e);
                false
            }
        }
    });
    bind!(main, on_add_files, |s| {
        s.dialog(5);
    });
    bind!(main, on_refresh_icon_ready, |s, value| {
        if let Ok((epoch, id, key, old, icon)) =
            serde_json::from_str::<(u64, i64, String, Vec<u8>, Vec<u8>)>(&value)
        {
            if epoch != s.data_epoch {
                return;
            }
            if icon.is_empty() {
                s.status("无法从目标提取图标，保留当前图标");
            } else if let Some(mut e) = s
                .entries
                .iter()
                .find(|e| e.id == id && e.target.duplicate_key() == key && e.icon == old)
                .cloned()
            {
                e.icon = icon;
                match s.store.save_entry(&e) {
                    Ok(_) => {
                        s.icon_cache.remove(&id);
                        s.report_refresh();
                        s.status("图标缓存已刷新");
                    }
                    Err(error) => s.status(error),
                }
            }
        }
    });
    bind!(main, on_icons_ready, |s, value| {
        if let Ok((epoch, icons)) =
            serde_json::from_str::<(u64, Vec<(i64, String, Vec<u8>)>)>(&value)
        {
            if epoch != s.data_epoch {
                return;
            }
            for (id, key, icon) in icons {
                if let Some(mut entry) = s
                    .entries
                    .iter()
                    .find(|e| e.id == id && e.icon.is_empty() && e.target.duplicate_key() == key)
                    .cloned()
                {
                    entry.icon = icon;
                    if let Err(e) = s.store.save_entry(&entry) {
                        s.status(e);
                    }
                }
            }
            s.report_refresh();
        }
    });
    bind!(main, on_icon_ready, |s, value| {
        if let Ok((epoch, requested_target, icon)) =
            serde_json::from_str::<(u64, String, Vec<u8>)>(&value)
            && epoch == s.icon_epoch
        {
            s.icon_pending = false;
            if let Some(session) = &mut s.editor {
                if session.window.get_target().as_str() != requested_target {
                    session.icon_attempted = false;
                    session.window.set_saving(false);
                    session.window.set_error("目标已更改，请重新保存".into());
                    s.saving_editor = false;
                    return;
                }
                session
                    .window
                    .set_entry_icon(diascia::icons::display(&icon));
                session.icon = icon;
                session.window.set_saving(false);
            }
            if s.saving_editor {
                s.saving_editor = false;
                s.save_editor();
            }
        }
    });
    bind!(main, on_files_ready, |s, value| {
        s.importing = false;
        let result = serde_json::from_str::<std::result::Result<Vec<Entry>, String>>(&value)
            .map_err(|e| e.to_string())
            .and_then(|r| r)
            .and_then(|mut entries| {
                if s.settings.relative_paths {
                    for entry in &mut entries {
                        if let Ok(t) =
                            diascia::paths::convert_target(&entry.target, &s.data_dir, true)
                        {
                            entry.target = t;
                        }
                    }
                }
                s.store.add_entries(&entries).map_err(|e| e.to_string())
            });
        match result {
            Ok((added, duplicates)) => {
                s.status(format!("已添加 {added} 项，跳过 {duplicates} 项重复目标"));
                s.report_refresh();
            }
            Err(e) => s.status(format!("未添加：{e}")),
        }
    });
    bind!(main, on_choose_group, |s, id| {
        if !s.finish_inline() {
            return;
        }
        s.remember_view();
        s.reset_batch_selection();
        s.group = i64::from(id);
        s.collection = s
            .browsing
            .group_categories
            .get(&s.group)
            .copied()
            .unwrap_or(s.group);
        if let Err(e) = s.rebuild_navigation() {
            s.status(e);
        }
    });
    bind!(main, on_choose_category, |s, id| {
        if !s.finish_inline() {
            return;
        }
        s.remember_view();
        s.reset_batch_selection();
        s.collection = s.category_for(i64::from(id));
        if let Err(e) = s.rebuild_navigation() {
            s.status(e);
        }
    });
    bind!(main, on_launch, |s, id| {
        if !s.finish_inline() {
            return;
        }
        if s.main.get_batch_mode() {
            let ids = s.displayed_ids();
            s.browsing
                .toggle(i64::from(id), platform::key_modifiers() & 4 != 0, &ids);
            s.update_selection();
        } else {
            s.launch(i64::from(id));
        }
    });
    bind!(main, on_launch_finished, |s, error| {
        s.main.set_busy(false);
        s.evaluate();
        if error.is_empty() {
            if s.settings.hide_after_launch && !s.auto_hide_guard().paused() {
                s.hide_all();
            }
        } else if let Ok(failure) = serde_json::from_str::<Failure>(&error) {
            s.status(failure.message.clone());
            s.show_relocate(failure.entry, &failure.message);
        } else {
            // A payload we cannot parse must still tell the user the launch failed.
            s.status(error);
        }
    });
    bind!(main, on_relocate, |s| {
        s.begin_relocate();
    });
    bind!(main, on_move, |s, id, direction| {
        if let Err(e) = s.store.move_entry(i64::from(id), direction) {
            s.status(e);
        }
        s.report_refresh();
    });
    bind!(main, on_remove, |s, id| {
        if s.editor
            .as_ref()
            .is_some_and(|e| e.original.id == i64::from(id))
        {
            s.status("请先关闭此项目的编辑窗口");
        } else {
            if let Err(e) = s.store.delete_entry(i64::from(id)) {
                s.status(e);
            }
            s.main.set_selected_id(0);
            s.report_refresh();
        }
    });
    bind!(main, on_view_change, |s, mode| {
        let mut settings = s.settings.clone();
        if mode == 3 {
            settings.list_view = true;
        } else if (0..=2).contains(&mode) {
            settings.icon_size = mode as u8;
            settings.list_view = false;
        } else {
            return;
        }
        match s.store.save_settings(&settings) {
            Ok(()) => {
                s.settings = settings;
                s.sync_main_settings();
            }
            Err(e) => s.status(e),
        }
    });
    bind!(main, on_pin_changed, |s| {
        s.apply_pin();
    });
    bind!(main, on_drag_start, |s| {
        s.reveal_edge();
        s.dock = None;
        platform::decorate(s.main.window());
        let p = s.main.window().position();
        s.drag = Some((platform::cursor(), (p.x, p.y)));
        // Undocked, `interaction_interval` would arm nothing, and the button-release that ends
        // the drag is only observed from a tick.
        s.schedule_interaction();
        // The Slint press callback is the authority. Delaying this until a timer
        // and polling GetAsyncKeyState can drop an entire quick startup drag.
        platform::begin_launcher_drag(s.main.window());
    });
    bind!(main, on_convert_path, |s, id, relative| {
        if s.editor.is_some() {
            s.status("请先完成当前编辑");
            return;
        }
        if let Some(mut entry) = s.entries.iter().find(|e| e.id == i64::from(id)).cloned() {
            match diascia::paths::convert_target(&entry.target, &s.data_dir, relative) {
                Ok(target) => {
                    entry.target = target;
                    if let Err(e) = s.store.save_entry(&entry) {
                        s.status(e);
                    }
                    s.report_refresh();
                }
                Err(e) => s.status(e),
            }
        }
    });
    let weak = Rc::downgrade(&app);
    main.on_change_icon(move |id| {
        if let Some(app) = weak.upgrade() {
            open_editor(&app, i64::from(id));
            let mut s = app.borrow_mut();
            if s.editor
                .as_ref()
                .is_some_and(|e| e.original.id == i64::from(id))
            {
                s.dialog(6);
            }
        }
    });
    bind!(main, on_settings, |s| {
        s.show_settings();
    });
    bind!(main, on_external_drop, |s, payload| {
        if !s.allowed(ShowSource::Editor) {
            s.main.set_drop_hover(false);
            return;
        }
        let destination = if s.main.get_external_category() != 0 {
            s.category_for(i64::from(s.main.get_external_category()))
        } else {
            s.collection
        };
        s.main.set_drop_hover(false);
        match serde_json::from_str::<diascia::importing::ExternalDrop>(&payload) {
            Ok(diascia::importing::ExternalDrop::Files(paths)) => {
                if paths.is_empty() {
                    s.status("每次最多添加 500 项");
                } else if let Err(e) = s.queue_files(paths, destination) {
                    s.status(e);
                }
            }
            Ok(diascia::importing::ExternalDrop::Text(text)) => {
                match diascia::importing::url_entries(&text, destination)
                    .and_then(|entries| s.store.add_entries(&entries))
                {
                    Ok((added, duplicates)) => {
                        s.status(format!("已添加 {added} 项，跳过 {duplicates} 项重复目标"));
                        s.report_refresh();
                        // Nothing restarts the launcher to pick these up, so the favicons are
                        // fetched here rather than by the startup pass.
                        let wanted: Vec<_> = s
                            .entries
                            .iter()
                            .filter(|e| e.icon.is_empty() && matches!(e.target, Target::Url { .. }))
                            .map(|e| (e.id, e.target.clone()))
                            .collect();
                        s.fill_icons(wanted);
                    }
                    Err(e) => s.status(e),
                }
            }
            Err(e) => s.status(e),
        }
    });
    let interaction_weak = main.as_weak();
    main.on_interaction_changed(move || {
        let weak = interaction_weak.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_native_event(8, "".into());
            }
        });
    });
    let viewport_pending = Rc::new(std::cell::Cell::new(false));
    let viewport_weak = main.as_weak();
    main.on_viewport_changed(move || {
        if viewport_pending.replace(true) {
            return;
        }
        let pending = viewport_pending.clone();
        let weak = viewport_weak.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            pending.set(false);
            if let Some(ui) = weak.upgrade() {
                ui.invoke_native_event(9, "".into());
            }
        });
    });
    bind!(main, on_search_scope_changed, |s| {
        s.reset_batch_selection();
        s.render_search();
        s.update_visible_icons();
    });
    bind!(main, on_batch_start, |s, _id| {
        if s.editor.is_some() || s.importing || s.file_dialog_open {
            return;
        }
        s.main.set_batch_mode(true);
        s.update_selection();
    });
    bind!(main, on_batch_all, |s| {
        if !s.main.get_batch_mode() {
            return;
        }
        if s.main.get_batch_all_selected() {
            s.browsing.clear_selection();
        } else {
            s.browsing.selected = s.displayed_ids().into_iter().collect();
        }
        s.update_selection();
    });
    bind!(main, on_batch_target, |s, action| {
        s.batch_target(action);
    });
    bind!(main, on_batch_node, |s, id| {
        s.batch_node(id);
    });
    bind!(main, on_batch_tree_key, |s, direction| {
        s.batch_tree_key(direction);
    });
    bind!(main, on_batch_cancel, |s| {
        s.clear_batch();
        s.update_selection();
    });
    bind!(main, on_batch_execute, |s, action, destination| {
        s.batch_action(action, destination);
    });
    bind!(main, on_request_hide, |s| {
        s.save_window(true);
        s.hide_all();
    });
    bind!(main, on_move_to, |s, id, destination| {
        if s.editor
            .as_ref()
            .is_some_and(|e| e.original.id == i64::from(id))
        {
            s.status("请先完成此项目的编辑");
        } else {
            let base = s.data_dir.clone();
            if let Err(e) = s
                .store
                .move_to_collection(i64::from(id), i64::from(destination), &base)
            {
                s.status(e);
            }
            s.report_refresh();
        }
    });
    bind!(main, on_quit, |s| {
        s.request_quit();
    });
    bind!(main, on_escape, |s| {
        if s.main.get_relocate_open() {
            s.main.set_relocate_open(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_batch_target_open() {
            s.main.set_batch_target_open(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_batch_confirm() {
            s.main.set_batch_confirm(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_more_open() {
            s.main.invoke_close_popups();
        } else if s.main.get_inline_kind() != 0 {
            s.cancel_inline();
        } else if s.main.get_conversion_open() {
            s.main.set_conversion_open(false);
        } else if s.main.get_collection_delete() {
            s.main.set_collection_delete(false);
        } else if s.main.get_confirm_delete() {
            s.main.set_confirm_delete(false);
        } else if s.main.get_batch_mode() {
            s.clear_batch();
            s.update_selection();
        } else if s.main.get_search_open() {
            s.main.set_search_open(false);
            s.main.set_query("".into());
            s.main.invoke_restore_grid_focus();
            s.render_items();
        } else if s.main.get_selected_id() != 0 {
            s.main.set_selected_id(0);
        } else {
            s.hide_all();
        }
    });
    bind!(main, on_native_event, |s, code, text| {
        if matches!(code, 1 | 3 | 4 | 6 | 7) && text.parse::<u64>().ok() != Some(s.policy.ticket())
        {
            return;
        }
        match code {
            1 => s.show_main(ShowSource::Hotkey),
            2 => s.status(text),
            3 => s.show_main(ShowSource::Tray),
            7 => s.show_main(ShowSource::SecondaryInstance),
            8 => {
                s.initialize_native_state();
                s.tick_window(false);
                s.schedule_interaction();
            }
            10 => {
                s.initialize_native_state();
                // SetWindowPos during animation generates Moved events. Feeding
                // those back into tick_window creates a frame-per-message loop.
                if s.edge_motion.is_none() {
                    s.tick_window(false);
                    s.schedule_interaction();
                }
            }
            9 => s.update_visible_icons(),
            14 | 17 => s.evaluate_suggested(),
            15 => {
                s.main.set_pinned(!s.main.get_pinned());
                s.apply_pin();
            }
            16 => s.toggle_protection_bypass(),
            12 => {
                if let Ok(generation) = text.parse() {
                    s.install_editor_drop(generation);
                }
            }
            13 => {
                if let Ok((generation, path)) = serde_json::from_str::<(u64, String)>(&text)
                    && s.editor
                        .as_ref()
                        .is_some_and(|e| e.generation == generation)
                    && !s.file_dialog_open
                    && !s.saving_editor
                    && let Err(e) = s.file_selected(6, path)
                    && let Some(editor) = &s.editor
                {
                    editor.window.set_error(e.to_string().into());
                }
            }
            11 => {
                if text
                    .parse()
                    .ok()
                    .is_some_and(|generation| s.frame_clock.acknowledge(generation))
                {
                    s.tick_window(true);
                    s.schedule_interaction();
                }
            }
            4 => s.show_settings(),
            5 => s.request_quit(),
            6 if s.allowed(ShowSource::Restore) => {
                s.show_main(ShowSource::Restore);
                s.dock = None;
                platform::restore_position(s.main.window());
                platform::decorate(s.main.window());
            }
            _ => {}
        }
    });
    bind!(main, on_file_selected, |s, kind, value| {
        if let Err(e) = s.file_selected(kind, value.to_string()) {
            s.status(&e);
            if let Some(ui) = s.settings_ui.get() {
                ui.set_status(e.to_string().into());
            }
            if let Some(editor) = &s.editor {
                editor.window.set_error(e.to_string().into());
            }
        }
    });
    bind!(main, on_manage_group, |s, id| {
        s.begin_collection_action(id, if id == 0 { 0 } else { 2 }, 0);
    });
    let weak = Rc::downgrade(&app);
    main.on_add(move || {
        if let Some(app) = weak.upgrade() {
            open_editor(&app, 0);
        }
    });
    let weak = Rc::downgrade(&app);
    main.on_edit(move |id| {
        if let Some(app) = weak.upgrade() {
            open_editor(&app, i64::from(id));
        }
    });
    let weak = Rc::downgrade(&app);
    main.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade() {
            app.borrow_mut().hide_all();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let weak = main.as_weak();
    let drop_weak = main.as_weak();
    let compose_weak = main.as_weak();
    install_keys(
        main.window(),
        move |key| {
            use slint::winit_030::winit::keyboard::NamedKey;
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            if ui.get_batch_target_open() || ui.get_batch_confirm() {
                if key == NamedKey::Escape {
                    ui.invoke_escape();
                    return true;
                }
                if ui.get_batch_confirm() && key == NamedKey::Enter {
                    ui.invoke_batch_execute(2, 0);
                    return true;
                }
                return matches!(key, NamedKey::Select | NamedKey::Find);
            }
            if key == NamedKey::Select && ui.get_batch_mode() && !ui.get_search_focused() {
                if !ui.get_batch_all_selected() {
                    ui.invoke_batch_all();
                }
                return true;
            }
            if key == NamedKey::Find {
                ui.invoke_focus_search();
                return true;
            }
            if key == NamedKey::Escape {
                ui.invoke_escape();
                return true;
            }
            if ui.get_search_focused() && matches!(key, NamedKey::ArrowUp | NamedKey::ArrowDown) {
                let items = ui.get_results();
                let count = items.row_count();
                if count == 0 {
                    return true;
                }
                let current = (0..count).find(|i| {
                    items
                        .row_data(*i)
                        .is_some_and(|item| item.id == ui.get_selected_id())
                });
                let next = if key == NamedKey::ArrowDown {
                    current.map_or(0, |i| (i + 1).min(count - 1))
                } else {
                    current.map_or(count - 1, |i| i.saturating_sub(1))
                };
                if let Some(item) = items.row_data(next) {
                    ui.set_selected_id(item.id);
                    ui.invoke_reveal_result(next as i32);
                }
                return true;
            }
            false
        },
        move |event| {
            use slint::winit_030::winit::event::WindowEvent;
            if let Some(ui) = drop_weak.upgrade() {
                match event {
                    WindowEvent::Moved(_) => {
                        let weak = ui.as_weak();
                        let _ =
                            weak.upgrade_in_event_loop(|ui| ui.invoke_native_event(10, "".into()));
                    }
                    WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::ScaleFactorChanged { .. } => {
                        let weak = ui.as_weak();
                        let _ =
                            weak.upgrade_in_event_loop(|ui| ui.invoke_native_event(8, "".into()));
                    }
                    // One timer and one closure per frame is not free, and without
                    // DIASCIA_METRICS it would be allocated only to be dropped. The guard is that
                    // check; `_` then covers a frame with nothing to measure.
                    WindowEvent::RedrawRequested if diascia::metrics::enabled() => {
                        let started = Instant::now();
                        slint::Timer::single_shot(Duration::ZERO, move || {
                            diascia::metrics::record("frame_dispatched", 0.);
                            diascia::metrics::record(
                                "frame_ms",
                                started.elapsed().as_secs_f64() * 1000.,
                            );
                        });
                    }
                    _ => {}
                }
            }
        },
        move |active| {
            if let Some(ui) = compose_weak.upgrade() {
                ui.set_composing(active);
            }
        },
        true,
    );
    bind_ui_commands(&app);
    {
        let mut s = app.borrow_mut();
        s.refresh()?;
        s.sync_settings_ui();
    }
    let startup = Rc::downgrade(&app);
    slint::Timer::single_shot(Duration::ZERO, move || {
        if let Some(app) = startup.upgrade() {
            app.borrow_mut().show_main(ShowSource::Startup);
        }
    });
    {
        let s = app.borrow();
        // Addresses are left out: a favicon costs a round trip, and a launch is the wrong moment to
        // pay for it. They are fetched when an entry is added or when the cache is refreshed.
        let wanted: Vec<_> = s
            .entries
            .iter()
            .filter(|e| e.icon.is_empty() && !matches!(e.target, Target::Url { .. }))
            .map(|e| (e.id, e.target.clone()))
            .collect();
        s.fill_icons(wanted);
    }

    let timer = slint::Timer::default();
    let weak = Rc::downgrade(&app);
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(500),
        move || {
            if let Some(app) = weak.upgrade() {
                let mut s = app.borrow_mut();
                s.evaluate();
                s.process_files();
                s.save_window(false);
                diascia::metrics::flush();
            }
        },
    );
    #[cfg(test)]
    app_regression::install(app.clone());
    slint::run_event_loop_until_quit()?;
    Ok(())
}
fn main() {
    diascia::metrics::init();
    // Slint and tray-icon share muda's process-global, single-assignment handler.
    // Application menus use TrackPopupMenuEx directly; widget popups stay in Slint
    // so neither path takes muda's tray handler. Set before worker threads exist.
    unsafe {
        std::env::set_var("SLINT_NO_MUDA", "1");
    }
    let message = match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return,
        Ok(Err(error)) => {
            eprintln!("Diascia: {error}");
            ("Diascia 无法启动", error.to_string())
        }
        Err(payload) => {
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "发生意外错误".into());
            diascia::diagnostics::error("fatal", &text);
            ("Diascia 运行中断", text)
        }
    };
    let detail = diascia::diagnostics::error_log_path()
        .map(|path| format!("\n详细记录：{}", path.display()))
        .unwrap_or_default();
    rfd::MessageDialog::new()
        .set_title(message.0)
        .set_description(format!("{}{detail}", message.1))
        .set_level(rfd::MessageLevel::Error)
        .show();
}

struct Deferred<T>(std::cell::OnceCell<T>);
impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self(std::cell::OnceCell::new())
    }
}
impl<T> Deferred<T> {
    fn get(&self) -> Option<&T> {
        self.0.get()
    }
}
impl<T> std::ops::Deref for Deferred<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get().expect("auxiliary window initialized before use")
    }
}
fn create_settings(
    app: &Rc<RefCell<App>>,
    data: &std::path::Path,
) -> std::result::Result<SettingsWindow, slint::PlatformError> {
    let settings_ui = SettingsWindow::new()?;
    settings_ui.set_rust_version(env!("DIASCIA_RUST_VERSION").into());
    settings_ui.set_data_path(data.to_string_lossy().to_string().into());
    settings_ui
        .window()
        .set_size(slint::LogicalSize::new(660., 480.));
    install_settings_keys(&settings_ui);
    macro_rules! bind { ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => { { let weak = Rc::downgrade(app); $ui.$callback(move |$($arg),*| { if let Some(app) = weak.upgrade() { #[allow(unused_mut)] let mut $s = app.borrow_mut(); $body } }); } }; }
    settings_ui.on_open_rust(|| {
        let _ = platform::launch(&Target::Url {
            url: "https://www.rust-lang.org/".into(),
        });
    });
    bind!(settings_ui, on_record_hotkey, |s| {
        s.settings_ui
            .set_recording_hotkey(!s.settings_ui.get_recording_hotkey());
        s.settings_ui.set_hotkey_error("".into());
        let enabled = !s.settings_ui.get_recording_hotkey() && s.policy.hotkey_enabled();
        s.hotkey.set_enabled(enabled);
        // A foreground request still goes through the policy, even from the settings window.
        if s.allowed(ShowSource::Settings) {
            platform::focus(s.settings_ui.window());
        }
    });
    bind!(settings_ui, on_reset_hotkey, |s| {
        s.settings_ui.set_recording_hotkey(false);
        s.settings_ui
            .set_hotkey_text(diascia::hotkey::DEFAULT.into());
        s.apply_settings(2);
        s.evaluate();
    });
    bind!(settings_ui, on_captured_hotkey, |s, value| {
        s.settings_ui.set_recording_hotkey(false);
        if !value.is_empty() {
            match diascia::hotkey::parse(&value) {
                Ok(shortcut) => {
                    s.settings_ui.set_hotkey_text(shortcut.label.into());
                    s.apply_settings(2);
                }
                Err(error) => {
                    s.settings_ui.set_hotkey_error(error.into());
                    s.settings_ui.set_recording_hotkey(true);
                }
            }
        }
        s.evaluate();
    });
    bind!(settings_ui, on_protection_bypass_changed, |s, enabled| {
        if s.policy.bypass() != enabled {
            s.toggle_protection_bypass();
        }
    });
    bind!(settings_ui, on_change, |s, field| {
        s.apply_settings(field);
    });
    bind!(settings_ui, on_browse_game, |s| {
        s.dialog(1);
    });
    bind!(settings_ui, on_add_protected_app, |s, value| {
        s.add_protected_app(value.to_string());
        s.settings_ui.set_protected_input("".into());
    });
    bind!(settings_ui, on_remove_protected_app, |s, index| {
        s.remove_protected(index);
    });
    bind!(settings_ui, on_export_data, |s| {
        s.dialog(2);
    });
    bind!(settings_ui, on_backup, |s| {
        s.dialog(3);
    });
    bind!(settings_ui, on_import_data, |s| {
        s.dialog(4);
    });
    bind!(settings_ui, on_confirm_restore, |s| {
        if let Err(e) = s.restore() {
            s.settings_ui.set_status(e.to_string().into());
        }
    });
    bind!(settings_ui, on_cancel, |s| {
        s.close_settings();
    });
    let weak = Rc::downgrade(app);
    settings_ui.window().on_close_requested(move || {
        if let Some(app) = weak.upgrade() {
            let mut s = app.borrow_mut();
            s.close_settings();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    Ok(settings_ui)
}
