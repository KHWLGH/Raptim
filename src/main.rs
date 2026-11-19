#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use raptim::{
    AppOption, BrokenOption, DestinationNode, DiscoverOption, EditorWindow, Item, MainWindow,
    NavItem, Palette, ScopeNode, SettingsWindow,
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
    time::{Duration, Instant},
};
mod app_session;
mod ui_commands;

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
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct BrokenEntry {
    id: i64,
    name: String,
    target: String,
    reason: String,
    relocatable: bool,
}
fn dragged_id(data: &slint::DataTransfer) -> Option<i64> {
    data.user_data()?
        .downcast_ref::<DraggedEntry>()
        .map(|v| v.0)
}
struct App {
    protection_resume: raptim::session::ProtectionResume,
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
    notices: std::cell::RefCell<raptim::session::NoticeQueue>,
    _instance: platform::Instance,
    initial: Option<Export>,
    data_epoch: u64,
    saved_window: raptim::session::WindowState,
    window_candidate: Option<(raptim::session::WindowState, Instant)>,
    browsing: raptim::session::Browsing,
    editor: Option<EditorSession>,
    settings_ui: Deferred<SettingsWindow>,
    settings_open: bool,
    installed_apps: Vec<(String, String)>,
    drag: Option<((i32, i32), (i32, i32))>,
    dock: Option<(raptim::docking::Edge, windowing::Rect)>,
    dock_overhang: Option<(raptim::docking::Edge, i32)>,
    edge_hidden: bool,
    /// Set by `collapsed_probe` when the pointer is outside the approach margin, so the next
    /// interaction tick can run at half rate.
    collapsed_far: bool,
    protected_edge_waiting: bool,
    dock_work: Option<windowing::Rect>,
    edge_progress: f32,
    edge_motion: Option<raptim::docking::Motion>,
    focus_hide: raptim::docking::FocusHide,
    edge_hold: raptim::docking::EdgeHold,
    edge_since: Instant,
    search_index: raptim::search::SearchIndex,
    icon_cache: raptim::icons::Cache,
    icon_index: std::collections::HashMap<i64, raptim::icons::IconKey>,
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
    conversion_preview: Option<raptim::commands::ConversionPreview>,
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
    silent_startup: bool,
    discovered_apps: Vec<platform::DiscoveredApp>,
    discovered_checked: Vec<bool>,
    broken_items: Vec<BrokenEntry>,
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
    fn auto_hide_guard(&self) -> raptim::policy::AutoHideGuard {
        raptim::policy::AutoHideGuard {
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
    fn dock_overhang_for(&self, edge: raptim::docking::Edge) -> i32 {
        match self.dock_overhang {
            Some((applied, pad)) if applied == edge => pad,
            _ => 0,
        }
    }
    fn sync_dock_overhang(&mut self) {
        let scale = self.main.window().scale_factor();
        let wanted = if self.settings.edge_hide {
            self.dock
                .map(|(edge, _)| (edge, raptim::docking::overhang_px(scale)))
        } else {
            None
        };
        let prev = self.dock_overhang;
        let size_matches = match (wanted, self.dock) {
            (Some((edge, pad)), Some((dock_edge, visible))) if edge == dock_edge => {
                let expected = raptim::docking::docked_hwnd_rect(edge, visible, pad);
                let size = self.main.window().size();
                size.width as i32 == expected.width && size.height as i32 == expected.height
            }
            (None, _) => prev.is_none(),
            _ => false,
        };
        if wanted == prev && size_matches {
            return;
        }
        self.dock_overhang = wanted;
        self.main.set_dock_edge(match wanted {
            Some((raptim::docking::Edge::Left, _)) => 0,
            Some((raptim::docking::Edge::Right, _)) => 1,
            Some((raptim::docking::Edge::Top, _)) => 2,
            None => -1,
        });
        self.main
            .set_dock_overhang(wanted.map_or(0., |(_, pad)| pad as f32 / scale));
        self.sync_edge_ui();
        if let Some((edge, pad)) = wanted
            && let Some((dock_edge, visible)) = self.dock
            && dock_edge == edge
        {
            let hwnd = raptim::docking::docked_hwnd_rect(edge, visible, pad);
            if self.edge_hidden || self.edge_motion.is_some() {
                platform::set_size_no_activate(self.main.window(), hwnd.width, hwnd.height);
            } else {
                platform::set_bounds(self.main.window(), hwnd);
                if let Some(work) = self.dock_work {
                    platform::move_clipped(self.main.window(), (hwnd.x, hwnd.y), work);
                }
            }
            windowing::invalidate(self.main.window());
            platform::request_redraw_now(self.main.window());
        } else if let Some((old_edge, old_pad)) = prev
            && let Some((actual, work)) = platform::geometry(self.main.window())
        {
            let mut visible = raptim::docking::visible_rect_from_hwnd(old_edge, actual, old_pad);
            visible.x = visible
                .x
                .clamp(work.x, work.x + (work.width - visible.width).max(0));
            visible.y = visible
                .y
                .clamp(work.y, work.y + (work.height - visible.height).max(0));
            platform::set_bounds(self.main.window(), visible);
            windowing::invalidate(self.main.window());
            platform::request_redraw_now(self.main.window());
        }
    }
    fn edge_strip_px(&self) -> i32 {
        if self.settings.edge_strip_visible {
            self.settings.edge_strip_width_px as i32
        } else {
            0
        }
    }
    fn sync_edge_ui(&self) {
        let scale = self.main.window().scale_factor().max(0.5);
        self.main.set_edge_progress(self.edge_progress);
        self.main
            .set_edge_strip_size(self.edge_strip_px() as f32 / scale);
    }
    fn apply_theme(&self) {
        let mode = self.settings.theme_mode;
        let changed = platform::apply_window_theme(self.main.window(), mode, false);
        if let Some(ui) = self.settings_ui.get() {
            platform::apply_window_theme(ui.window(), mode, changed);
        }
        if let Some(editor) = &self.editor {
            platform::apply_window_theme(editor.window.window(), mode, changed);
        }
    }
    fn sync_edge_appearance(&self) {
        self.sync_edge_ui();
        let appearance = raptim::docking::collapsed_appearance(
            self.dock.is_some() && self.settings.edge_hide,
            self.edge_hidden,
            self.edge_progress,
            self.edge_motion.is_some(),
            self.edge_strip_px(),
        );
        platform::set_collapsed_appearance(self.main.window(), appearance);
    }
    /// Appearance only: un-fade and slide the window back to its docked rectangle. Refuses to
    /// touch the window while any protection holds, so a background tick can never reveal it.
    fn reveal_edge(&mut self) {
        if self.policy.protection() != Protection::Normal {
            return;
        }
        let displaced = self.edge_hidden || self.edge_progress > 0.;
        self.edge_motion = None;
        self.edge_progress = 0.;
        self.edge_hidden = false;
        self.focus_hide.reset();
        platform::set_opacity(self.main.window(), 1.);
        self.sync_edge_appearance();
        if displaced && let Some((edge, r)) = self.dock {
            let pad = self.dock_overhang_for(edge);
            let hwnd = raptim::docking::docked_hwnd_rect(edge, r, pad);
            if pad > 0
                && let Some(work) = self.dock_work
            {
                platform::move_clipped(self.main.window(), (hwnd.x, hwnd.y), work);
                platform::finish_edge_motion(self.main.window());
            } else {
                self.main
                    .window()
                    .set_position(slint::PhysicalPosition::new(hwnd.x, hwnd.y));
            }
            platform::decorate(self.main.window());
            windowing::invalidate(self.main.window());
            self.apply_theme();
            platform::topmost(self.main.window(), self.wants_topmost());
        }
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
        self.edge_progress = 1.;
        self.edge_hidden = true;
        self.edge_hold.clear();
        self.sync_edge_appearance();
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
        if hidden {
            self.edge_hold.clear();
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
        self.edge_motion = Some(raptim::docking::Motion::new(
            position,
            velocity,
            hidden,
            now,
            platform::animations_enabled(),
        ));
        self.sync_edge_appearance();
        if !hidden {
            self.apply_theme();
        }
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
            self.edge_hold.clear();
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
                    && let Some((raw, work)) = platform::geometry(self.main.window())
                {
                    let r = if let Some((edge, pad)) = self.dock_overhang {
                        raptim::docking::visible_rect_from_hwnd(edge, raw, pad)
                    } else {
                        raw
                    };
                    self.dock = raptim::docking::dock(
                        r,
                        work,
                        (14. * self.main.window().scale_factor()) as i32,
                    );
                    self.dock_work = Some(work);
                    self.sync_dock_overhang();
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
            && let Some((edge, old)) = self.dock
            && let Some((actual, work)) = platform::geometry(self.main.window())
        {
            let expected =
                raptim::docking::docked_hwnd_rect(edge, old, self.dock_overhang_for(edge));
            if actual.x != expected.x || actual.y != expected.y {
                let visible = raptim::docking::visible_rect_from_hwnd(
                    edge,
                    actual,
                    self.dock_overhang_for(edge),
                );
                self.dock = raptim::docking::dock(visible, work, 0);
                self.dock_work = Some(work);
                self.edge_since = now;
                self.sync_dock_overhang();
            }
        }
        if let Some((edge, old)) = self.dock {
            let size = self.main.window().size();
            let scale = self.main.window().scale_factor();
            let applied_pad = self.dock_overhang_for(edge);
            let wanted_pad = if self.settings.edge_hide {
                raptim::docking::overhang_px(scale)
            } else {
                0
            };
            let expected = raptim::docking::docked_hwnd_rect(edge, old, applied_pad);
            if let Some(work) =
                platform::work_area_at((old.x + old.width / 2, old.y + old.height / 2))
                && (Some(work) != self.dock_work
                    || size.width as i32 != expected.width
                    || size.height as i32 != expected.height
                    || applied_pad != wanted_pad)
            {
                let stay_hidden = self.protected_edge_waiting || self.edge_hidden;
                if !stay_hidden {
                    self.reveal_edge();
                }
                let raw_w = size.width as i32;
                let raw_h = size.height as i32;
                let (vis_w, vis_h) = match edge {
                    raptim::docking::Edge::Left | raptim::docking::Edge::Right => {
                        ((raw_w - applied_pad).max(1), raw_h.max(1))
                    }
                    raptim::docking::Edge::Top => (raw_w.max(1), (raw_h - applied_pad).max(1)),
                };
                let mut rect = windowing::Rect {
                    width: vis_w.min(work.width.max(1)),
                    height: vis_h.min(work.height.max(1)),
                    ..old
                };
                rect.x = rect
                    .x
                    .clamp(work.x, work.x + (work.width - rect.width).max(0));
                rect.y = rect
                    .y
                    .clamp(work.y, work.y + (work.height - rect.height).max(0));
                match edge {
                    raptim::docking::Edge::Left => rect.x = work.x,
                    raptim::docking::Edge::Right => rect.x = work.x + work.width - rect.width,
                    raptim::docking::Edge::Top => rect.y = work.y,
                }
                self.dock = Some((edge, rect));
                self.dock_work = Some(work);
                self.sync_dock_overhang();
                let pad = self.dock_overhang_for(edge);
                if stay_hidden {
                    let strip = self.edge_strip_px();
                    platform::move_clipped(
                        self.main.window(),
                        raptim::docking::hidden_hwnd_position(edge, rect, strip, pad),
                        work,
                    );
                    self.sync_edge_appearance();
                } else {
                    let hwnd = raptim::docking::docked_hwnd_rect(edge, rect, pad);
                    platform::set_bounds(self.main.window(), hwnd);
                    platform::move_clipped(self.main.window(), (hwnd.x, hwnd.y), work);
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
                    .is_some_and(|(rect, _)| raptim::docking::contains(rect, cursor)));
        if paused {
            if self.edge_progress > 0. || self.edge_motion.is_some() {
                self.animate_edge(false);
            } else {
                self.edge_hold.refresh_pending(now);
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
                self.edge_hold.clear();
                self.sync_dock_overhang();
                platform::decorate(self.main.window());
                return;
            }
            let strip = self.edge_strip_px();
            let band = raptim::docking::reveal_band(edge, r, strip);
            let pointer_inside = raptim::docking::contains(r, cursor);
            if paused {
                self.edge_hold.refresh_pending(now);
                self.edge_since = now;
            } else if self.edge_hidden {
                self.edge_hold.clear();
                if raptim::docking::contains(band, cursor)
                    || (self.edge_motion.is_some()
                        && platform::geometry(self.main.window()).is_some_and(|(actual, _)| {
                            raptim::docking::contains(actual, cursor)
                                && self
                                    .dock_work
                                    .is_some_and(|work| raptim::docking::contains(work, cursor))
                        }))
                {
                    if self.edge_since.elapsed() >= Duration::from_millis(24)
                        && self.allowed(ShowSource::Hotkey)
                    {
                        self.animate_edge(false);
                        self.edge_since = now;
                    }
                } else {
                    self.edge_since = now;
                }
            } else {
                let holding = self.edge_hold.is_active()
                    && self.edge_hold.hold(
                        pointer_inside,
                        self.edge_motion.is_some(),
                        platform::app_has_focus(),
                        now,
                    );
                if pointer_inside || holding || self.edge_motion.is_some() {
                    // Stay open while the pointer is inside, an explicit summon holds focus,
                    // or a reveal slide is still finishing.
                    self.edge_since = now;
                } else if self.edge_since.elapsed()
                    >= Duration::from_millis(u64::from(self.settings.edge_collapse_delay_ms))
                {
                    self.animate_edge(true);
                    self.edge_since = now;
                }
            }
            if advance_motion && let Some(motion) = self.edge_motion {
                let pad = self.dock_overhang_for(edge);
                let start = raptim::docking::docked_hwnd_rect(edge, r, pad);
                let end = raptim::docking::hidden_hwnd_position(edge, r, strip, pad);
                let (value, sampled_done) = motion.value(now);
                let target_pos = if self.edge_hidden {
                    end
                } else {
                    (start.x, start.y)
                };
                let x = (start.x as f32 + (end.0 - start.x) as f32 * value).round() as i32;
                let y = (start.y as f32 + (end.1 - start.y) as f32 * value).round() as i32;
                let done =
                    sampled_done || ((x, y) == target_pos && (value - motion.to).abs() < 0.015);
                self.edge_progress = if done { motion.to } else { value };
                let pos = if done { target_pos } else { (x, y) };
                if let Some(work) = self.dock_work {
                    platform::move_clipped(self.main.window(), pos, work);
                }
                if done {
                    self.edge_motion = None;
                    if !self.edge_hidden {
                        platform::finish_edge_motion(self.main.window());
                        platform::topmost(self.main.window(), self.wants_topmost());
                    }
                }
                self.sync_edge_appearance();
                if self.edge_progress > 0.60 || done {
                    platform::request_redraw_now(self.main.window());
                }
            }
        } else {
            self.edge_hold.clear();
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
        let strip = self.edge_strip_px();
        let cursor = platform::cursor();
        if raptim::docking::contains(raptim::docking::reveal_band(edge, rect, strip), cursor) {
            if self.edge_since.elapsed() >= Duration::from_millis(24)
                && self.allowed(ShowSource::Hotkey)
            {
                self.animate_edge(false);
                self.edge_since = now;
            }
        } else {
            self.edge_since = now;
            let work = self.dock_work.unwrap_or(rect);
            let approach =
                raptim::docking::approach_band(edge, work, strip, Self::APPROACH_MARGIN_PX);
            self.collapsed_far = !raptim::docking::contains(approach, cursor);
        }
        true
    }

    fn convert_editor_path(&mut self, relative: bool) {
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
        let ids: std::collections::HashSet<_> = self.entries.iter().map(|e| e.id).collect();
        self.icon_cache.retain(&ids);
        self.icon_index = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, e)| {
                (
                    e.id,
                    raptim::icons::IconKey {
                        fingerprint: raptim::icons::fingerprint(&e.icon),
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
            tooltip_path: raptim::paths::display_target(&e.target, &self.data_dir).into(),
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
                    raptim::session::SearchScope::from(scope).includes(
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
        let mut signals = platform::collect_signals(&self.settings, &mut self.protection_cache);
        signals.auxiliary_open = self.auto_hide_guard().auxiliary_open();
        signals.bypass = self.policy.bypass();
        let hold = signals.hold_release();
        self.apply_protection(decide(signals), Instant::now(), hold);
        self.last_evaluated = Instant::now();
    }
    /// Win-event and display broadcasts are only hints that something may have changed, and a
    /// fullscreen video can deliver them far faster than a shell query costs.
    fn evaluate_suggested(&mut self) {
        if self.last_evaluated.elapsed() < Duration::from_millis(120) {
            return;
        }
        self.evaluate();
    }
    fn apply_protection(&mut self, signal: Protection, now: Instant, hold: bool) {
        let previous = self.policy.protection();
        let changed = self.policy.update(signal, now, hold);
        if changed {
            raptim::diagnostics::protection(
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
        let label = self.settings.hotkey.clone();
        self.hotkey.set_enabled(
            &label,
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
            raptim::diagnostics::protection(
                "blocked",
                &format!("{source:?}"),
                &format!("{:?}", self.policy.protection()),
                &format!("ticket={ticket} current={}", self.policy.ticket()),
            );
        }
        allowed
    }
    fn dismiss_search_state(&mut self) {
        if !self.main.get_search_open() {
            return;
        }
        self.main.set_search_open(false);
        self.main.set_query("".into());
        self.search_was_open = false;
        self.render_search();
        self.main.invoke_restore_grid_focus();
    }
    fn hide_all(&mut self) {
        self.protected_edge_waiting = false;
        self.protection_resume.cancel();
        self.main.set_keyboard_focus(false);
        self.dismiss_search_state();
        self.hide_windows();
    }
    fn hide_windows(&mut self) {
        self.save_window(true);
        self.edge_hold.clear();
        self.interaction_timer.stop();
        self.frame_clock.set_active(false);
        windowing::close_popups(self.main.window());
        if let Some(ui) = self.settings_ui.get() {
            windowing::close_popups(ui.window());
        }
        let _ = self.main.hide();
        platform::set_collapsed_appearance(
            self.main.window(),
            raptim::docking::CollapsedAppearance::Normal,
        );
        platform::set_opacity(self.main.window(), 1.);
        platform::topmost(self.main.window(), false);
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
        let silent = std::mem::take(&mut self.silent_startup);
        if !self.allowed(source) {
            return;
        }
        if source == ShowSource::Startup && silent {
            if self.settings.edge_hide
                && let Some(edge) = self.saved_window.edge.and_then(|e| match e {
                    0 => Some(raptim::docking::Edge::Left),
                    1 => Some(raptim::docking::Edge::Right),
                    2 => Some(raptim::docking::Edge::Top),
                    _ => None,
                })
            {
                platform::restore_saved(self.main.window(), &self.saved_window);
                if let Some((rect, work)) = platform::geometry(self.main.window()) {
                    self.dock = Some((edge, rect));
                    self.dock_work = Some(work);
                    self.edge_motion = None;
                    self.edge_hidden = true;
                    self.edge_hold.clear();
                    self.edge_progress = 1.;
                    self.position_restored = true;
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
                    platform::topmost(self.main.window(), true);
                    platform::launcher_window(self.main.window());
                    self.sync_edge_appearance();
                    let _ = platform::show_without_activation(self.main.window());
                    self.apply_theme();
                    self.install_external_drop();
                    windowing::invalidate(self.main.window());
                    self.schedule_interaction();
                }
            }
            return;
        }
        let visible_edge = self.main.window().is_visible()
            && self.dock.is_some()
            && (self.edge_progress > 0. || self.edge_motion.is_some());
        if !self.main.window().is_visible() {
            if platform::native_ready(self.main.window()) {
                platform::topmost(self.main.window(), self.wants_topmost());
            }
            if let Err(e) = windowing::show(self.main.window()) {
                self.status(e);
                return;
            }
        }
        self.initialize_native_state();
        if visible_edge && !self.auto_hide_guard().auxiliary_open() {
            self.animate_edge(false);
            self.edge_since = Instant::now();
        } else {
            self.user_summon_edge();
        }
        self.schedule_interaction();
        platform::launcher_window(self.main.window());
        platform::topmost(
            self.main.window(),
            self.edge_motion.is_some() || self.wants_topmost(),
        );
        platform::focus(self.main.window());
        self.focus_hide.reset();
        if self.dock.is_some() && self.settings.edge_hide && !self.edge_hidden {
            self.edge_hold
                .arm(Instant::now(), platform::app_has_focus());
        }
        if self.settings_open {
            let _ = windowing::show_child(self.settings_ui.window(), self.main.window());
        }
        if let Some(editor) = &self.editor {
            let _ = windowing::show_child(editor.window.window(), self.main.window());
        }
        self.apply_theme();
    }
    fn show_settings(&mut self) {
        if !self.allowed(ShowSource::Settings) {
            return;
        }
        self.user_summon_edge();
        if self.settings_ui.get().is_none() {
            match create_settings(
                &self.weak_self.upgrade().expect("live application"),
                self.main.as_weak(),
                &self.data_dir,
            ) {
                Ok(ui) => {
                    let _ = self.settings_ui.0.set(ui);
                }
                Err(e) => {
                    self.status(e);
                    return;
                }
            }
        }
        self.sync_settings_ui();
        self.show_main(ShowSource::Settings);
        self.settings_open = true;
        if let Err(error) = windowing::show_child(self.settings_ui.window(), self.main.window()) {
            self.settings_open = false;
            self.status(error);
        } else {
            self.apply_theme();
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
        self.main.set_type_to_search(self.settings.type_to_search);
        self.main.set_icon_size(i32::from(self.settings.icon_size));
        self.main.set_list_view(self.settings.list_view);
        self.main.set_hover_switch(self.settings.hover_switch);
        self.main
            .set_hover_delay(self.settings.hover_delay_ms as i32);
        self.sync_edge_ui();
        self.tray
            .set_tooltip(&format!("Raptim · {}", self.settings.hotkey));
    }
    fn sync_settings_ui(&self) {
        self.sync_main_settings();
        if self.settings_ui.get().is_none() {
            return;
        }
        self.settings_ui
            .set_hotkey_text(self.settings.hotkey.clone().into());
        self.settings_ui.set_protected_error("".into());
        self.settings_ui
            .set_theme_mode(i32::from(self.settings.theme_mode));
        self.settings_ui.set_autostart(self.settings.autostart);
        self.settings_ui.set_autostart_warning(
            platform::autostart_mismatch(&self.data_dir, self.settings.autostart)
                .unwrap_or_default()
                .into(),
        );
        self.settings_ui
            .set_double_click(self.settings.double_click);
        self.settings_ui
            .set_type_to_search(self.settings.type_to_search);
        self.settings_ui.set_edge_hide(self.settings.edge_hide);
        self.settings_ui
            .set_keep_topmost(self.saved_window.always_on_top);
        self.settings_ui
            .set_edge_strip_visible(self.settings.edge_strip_visible);
        self.settings_ui
            .set_edge_strip_width(self.settings.edge_strip_width_px as i32);
        self.settings_ui
            .set_edge_collapse_delay(self.settings.edge_collapse_delay_ms as i32);
        self.settings_ui
            .set_hide_on_focus_loss(self.settings.hide_on_focus_loss);
        self.settings_ui
            .set_hover_switch(self.settings.hover_switch);
        self.settings_ui
            .set_hover_delay(self.settings.hover_delay_ms as i32);
        self.settings_ui
            .set_relative_paths(self.settings.relative_paths);
        self.settings_ui
            .set_hide_after_launch(self.settings.hide_after_launch);
        self.settings_ui
            .set_protection_log(self.settings.protection_log);
        self.settings_ui.set_protection_bypass(self.policy.bypass());
        self.settings_ui
            .set_fullscreen_protection(self.settings.fullscreen_protection);
        self.settings_ui
            .set_protected_apps(strings(self.settings.protected_apps.clone()));
        self.settings_ui
            .set_has_error_log(raptim::diagnostics::error_log_path().is_some_and(|p| p.exists()));
    }
    fn settings_value(&self) -> Settings {
        let mut draft = self.settings.clone();
        draft.hotkey = self.settings_ui.get_hotkey_text().to_string();
        draft.theme_mode = self.settings_ui.get_theme_mode().clamp(0, 2) as u8;
        draft.autostart = self.settings_ui.get_autostart();
        draft.double_click = self.settings_ui.get_double_click();
        draft.type_to_search = self.settings_ui.get_type_to_search();
        draft.edge_hide = self.settings_ui.get_edge_hide();
        draft.edge_strip_visible = self.settings_ui.get_edge_strip_visible();
        draft.edge_strip_width_px = self.settings_ui.get_edge_strip_width() as u32;
        draft.edge_collapse_delay_ms = self.settings_ui.get_edge_collapse_delay() as u32;
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
        if let Err(e) = raptim::storage::validate_settings(&draft) {
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
                    self.edge_hold.clear();
                }
                self.settings = draft;
                self.sync_dock_overhang();
                if !self.settings.edge_hide {
                    platform::decorate(self.main.window());
                }
                raptim::diagnostics::set_protection_log(self.settings.protection_log);
                self.settings_ui.set_hotkey_error("".into());
                self.settings_ui.set_autostart_warning(
                    platform::autostart_mismatch(&self.data_dir, self.settings.autostart)
                        .unwrap_or_default()
                        .into(),
                );
                self.sync_main_settings();
                self.apply_theme();
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
        let mut target = raptim::paths::resolve_for_shell(&entry.target, &self.data_dir);
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
    fn show_relocate(&mut self, id: i64, message: &str, error_relocatable: bool) -> bool {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return false;
        };
        // Two independent questions, and the shipped build asked only one of them. Does this class
        // of failure mean "the path we hold does not resolve" — as opposed to a declined elevation,
        // a refused permission or an unclassified system error? And does this target have a path to
        // re-point at all?
        if !error_relocatable || !entry.target.relocatable() {
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
        self.broken_items.retain(|item| item.id != id);
        if self.main.get_broken_open() {
            self.sync_broken_ui();
        }
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
                    .add_filter("Raptim 导出", &["json"])
                    .set_file_name("raptim-export.json")
                    .save_file(),
                7 if folder => dialog.pick_folder(),
                7 => dialog.pick_file(),
                8 => dialog.pick_folder(),
                _ => dialog.add_filter("Raptim 导出", &["json"]).pick_file(),
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
                let icon = raptim::icons::from_file(&path)?;
                self.icon_epoch += 1;
                self.icon_pending = false;
                if let Some(session) = &mut self.editor {
                    session.window.set_entry_icon(raptim::icons::display(&icon));
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
                    let is_shortcut = path.extension().is_some_and(|e| {
                        e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url")
                    });
                    if !session.window.get_editing() {
                        let detected = if path.is_dir() {
                            2
                        } else if is_shortcut {
                            4
                        } else if path.extension().is_some_and(|e| {
                            e.eq_ignore_ascii_case("exe")
                                || e.eq_ignore_ascii_case("bat")
                                || e.eq_ignore_ascii_case("cmd")
                                || e.eq_ignore_ascii_case("msi")
                                || e.eq_ignore_ascii_case("com")
                        }) {
                            0
                        } else {
                            1
                        };
                        session.window.set_kind(detected);
                    } else if is_shortcut {
                        session.window.set_kind(4);
                    }
                }
            }
            8 => {
                if let Some(session) = &self.editor {
                    session.window.set_working_dir(value.into());
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
                raptim::storage::validate_export(&data)?;
                // `validate_export` answers about the raw value, so an export stays legal whatever
                // directory it was written on. This answers about the identity the rows are about to
                // be indexed under: two spellings of one file pass the first gate and land on a
                // single `dup_key`, which makes both of them uneditable from then on.
                if let Some(name) = self.store.export_clash(&data.entries) {
                    return Err(Error::Invalid(format!(
                        "同一分类内有指向同一目标的不同写法：{name}；未改动数据"
                    )));
                }
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
            let result = raptim::importing::file_entries(paths).map_err(|e| e.to_string());
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
        self.apply_theme();
        self.refresh()?;
        self.pending_import = None;
        self.settings_ui.set_restore_confirm(false);
        self.settings_ui.set_status("已恢复".into());
        self.evaluate();
        Ok(())
    }
    fn should_poll_tick(&self) -> bool {
        self.main.window().is_visible()
            || self
                .settings_ui
                .get()
                .is_some_and(|ui| ui.window().is_visible())
            || self
                .editor
                .as_ref()
                .is_some_and(|e| e.window.window().is_visible())
            || self.policy.protection() != Protection::Normal
            || self.protected_edge_waiting
    }
    fn toggle_hotkey_visibility(&mut self) {
        let can_toggle_off = self.main.window().is_visible()
            && !self.edge_hidden
            && self.edge_progress == 0.
            && self.edge_motion.is_none()
            && platform::app_has_focus()
            && self.editor.is_none()
            && !self.settings_open
            && !self.file_dialog_open
            && !self.main.get_interacting()
            && self.main.get_inline_kind() == 0;
        if can_toggle_off {
            if self.dock.is_some() && self.settings.edge_hide && !self.main.get_pinned() {
                self.dismiss_search_state();
                self.main.set_keyboard_focus(false);
                self.animate_edge(true);
                self.edge_since = Instant::now();
                self.schedule_interaction();
            } else {
                self.hide_all();
            }
        } else {
            self.show_main(ShowSource::Hotkey);
        }
    }
    fn step_category(&mut self, step: i32) {
        if !self.finish_inline() {
            return;
        }
        let mut ordered = Vec::new();
        for group in self.collections.iter().filter(|c| c.parent_id.is_none()) {
            for cat in self
                .collections
                .iter()
                .filter(|c| c.parent_id == Some(group.id))
            {
                ordered.push((group.id, cat.id));
            }
        }
        if ordered.len() <= 1 {
            return;
        }
        let current = ordered
            .iter()
            .position(|&(g, c)| g == self.group && c == self.collection)
            .unwrap_or(0);
        let next = if step >= 0 {
            (current + 1) % ordered.len()
        } else {
            (current + ordered.len() - 1) % ordered.len()
        };
        let (group, category) = ordered[next];
        self.remember_view();
        self.reset_batch_selection();
        self.group = group;
        self.collection = category;
        self.browsing.group_categories.insert(group, category);
        self.main.set_keyboard_focus(false);
        if let Err(e) = self.rebuild_navigation() {
            self.status(e);
        }
    }
    fn begin_discover_apps(&mut self) {
        if self.main.get_busy() {
            return;
        }
        if !self.finish_inline() {
            return;
        }
        self.user_summon_edge();
        self.discovered_apps.clear();
        self.discovered_checked.clear();
        self.main
            .set_discover_target_name(self.collection_name(self.collection));
        self.main.set_discover_filter("".into());
        self.main.set_discover_items(ModelRc::default());
        self.main.set_discover_selected_count(0);
        self.main.set_discover_loading(true);
        self.main.set_discover_open(true);
        self.main.invoke_focus_discover_filter();

        let existing: std::collections::HashSet<String> = self
            .entries
            .iter()
            .filter(|e| e.collection_id == self.collection)
            .map(|e| raptim::paths::resolve_for_shell(&e.target, &self.data_dir).duplicate_key())
            .collect();
        let data_dir = self.data_dir.clone();
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let mut apps = platform::discover_installed_entries();
            apps.retain(|app| {
                let key = raptim::paths::resolve_for_shell(&app.target, &data_dir).duplicate_key();
                !existing.contains(&key)
            });
            let payload = serde_json::to_string(&apps).unwrap_or_else(|_| "[]".into());
            let _ =
                weak.upgrade_in_event_loop(move |ui| ui.invoke_native_event(18, payload.into()));
        });
    }
    fn render_discover_list(&mut self) {
        let needle = self.main.get_discover_filter().trim().to_lowercase();
        let mut options = Vec::new();
        let mut selected_count = 0i32;
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if !needle.is_empty()
                && !app.name.to_lowercase().contains(&needle)
                && !app.detail.to_lowercase().contains(&needle)
            {
                continue;
            }
            let checked = self.discovered_checked.get(index).copied().unwrap_or(false);
            if checked {
                selected_count += 1;
            }
            options.push(DiscoverOption {
                checked,
                index: index as i32,
                name: app.name.clone().into(),
                detail: app.detail.clone().into(),
                kind: app.target.kind_index(),
            });
        }
        self.main.set_discover_selected_count(selected_count);
        self.main
            .set_discover_items(Rc::new(VecModel::from(options)).into());
    }
    fn toggle_discovered(&mut self, idx: i32) {
        let Ok(index) = usize::try_from(idx) else {
            return;
        };
        if let Some(flag) = self.discovered_checked.get_mut(index) {
            *flag = !*flag;
            self.render_discover_list();
        }
    }
    fn select_all_discovered(&mut self, checked: bool) {
        let needle = self.main.get_discover_filter().trim().to_lowercase();
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if (needle.is_empty()
                || app.name.to_lowercase().contains(&needle)
                || app.detail.to_lowercase().contains(&needle))
                && let Some(flag) = self.discovered_checked.get_mut(index)
            {
                *flag = checked;
            }
        }
        self.render_discover_list();
    }
    fn import_discovered(&mut self) {
        if self.discovered_apps.is_empty() {
            return;
        }
        let needle = self.main.get_discover_filter().trim().to_lowercase();
        let mut new_entries = Vec::new();
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if !self.discovered_checked.get(index).copied().unwrap_or(false) {
                continue;
            }
            if !needle.is_empty()
                && !app.name.to_lowercase().contains(&needle)
                && !app.detail.to_lowercase().contains(&needle)
            {
                continue;
            }
            let mut target = app.target.clone();
            if self.settings.relative_paths
                && let Ok(rel) = raptim::paths::convert_target(&target, &self.data_dir, true)
            {
                target = rel;
            }
            new_entries.push(Entry {
                id: 0,
                collection_id: self.collection,
                name: app.name.clone(),
                aliases: String::new(),
                notes: String::new(),
                target,
                position: 0,
                icon: Vec::new(),
            });
        }
        if new_entries.is_empty() {
            self.status("请先勾选要导入的软件");
            return;
        }
        match self.store.add_entries(&new_entries) {
            Ok((added, skipped)) => {
                self.main.set_discover_open(false);
                self.main.invoke_focus_browser();
                self.report_refresh();
                let wanted: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|e| {
                        e.collection_id == self.collection
                            && e.icon.is_empty()
                            && !matches!(e.target, Target::Url { .. })
                    })
                    .map(|e| (e.id, e.target.clone()))
                    .collect();
                self.fill_icons(wanted);
                self.status(if skipped > 0 {
                    format!("已导入 {added} 项，跳过 {skipped} 项重复")
                } else {
                    format!("已导入 {added} 项软件")
                });
            }
            Err(e) => self.status(e),
        }
    }
    fn begin_broken_check(&mut self) {
        if !self.finish_inline() {
            return;
        }
        self.user_summon_edge();
        self.broken_items.clear();
        self.main.set_broken_items(ModelRc::default());
        self.main.set_broken_loading(true);
        self.main.set_broken_open(true);
        let entries = self.entries.clone();
        let data_dir = self.data_dir.clone();
        let epoch = self.data_epoch;
        let weak = self.main.as_weak();
        std::thread::spawn(move || {
            let installed: std::collections::HashSet<String> = platform::installed_apps()
                .into_iter()
                .map(|(aumid, _)| aumid.to_lowercase())
                .collect();
            let mut found = Vec::new();
            for entry in entries {
                let resolved = raptim::paths::resolve_for_shell(&entry.target, &data_dir);
                let reason = match &resolved {
                    Target::Url { .. } => None,
                    Target::App { aumid } => {
                        if installed.contains(&aumid.to_lowercase()) {
                            None
                        } else {
                            Some("未找到对应的已安装应用")
                        }
                    }
                    Target::Folder { path } => {
                        let p = std::path::Path::new(path);
                        let low = path.to_ascii_lowercase();
                        if low.starts_with("shell:") || low.starts_with("::{") || p.is_dir() {
                            None
                        } else {
                            Some("目标文件夹不存在")
                        }
                    }
                    Target::Program { path, .. }
                    | Target::File { path }
                    | Target::Shortcut { path, .. } => {
                        let p = std::path::Path::new(path);
                        let low = path.to_ascii_lowercase();
                        let on_path =
                            matches!(resolved, Target::Program { .. }) && command_exists(path);
                        if low.starts_with("shell:")
                            || low.starts_with("::{")
                            || p.exists()
                            || on_path
                        {
                            None
                        } else {
                            Some("目标文件或快捷方式不存在")
                        }
                    }
                };
                if let Some(reason) = reason {
                    found.push(BrokenEntry {
                        id: entry.id,
                        name: entry.name,
                        target: entry.target.value().to_string(),
                        reason: reason.to_string(),
                        relocatable: entry.target.relocatable(),
                    });
                }
            }
            let payload = serde_json::to_string(&(epoch, found)).unwrap_or_default();
            let _ =
                weak.upgrade_in_event_loop(move |ui| ui.invoke_native_event(19, payload.into()));
        });
    }
    fn sync_broken_ui(&self) {
        let items: Vec<_> = self
            .broken_items
            .iter()
            .map(|b| BrokenOption {
                id: b.id as i32,
                name: b.name.clone().into(),
                target: b.target.clone().into(),
                reason: b.reason.clone().into(),
                relocatable: b.relocatable,
            })
            .collect();
        self.main
            .set_broken_items(Rc::new(VecModel::from(items)).into());
    }
    fn remove_broken_entry(&mut self, id: i64) {
        if self.editor.as_ref().is_some_and(|e| e.original.id == id) {
            self.status("请先关闭此项目的编辑窗口");
            return;
        }
        if let Err(e) = self.store.delete_entry(id) {
            self.status(e);
            return;
        }
        self.broken_items.retain(|item| item.id != id);
        self.sync_broken_ui();
        if self.main.get_selected_id() == id as i32 {
            self.main.set_selected_id(0);
        }
        self.report_refresh();
    }
}

fn command_exists(command: &str) -> bool {
    let trimmed = command.trim().trim_matches('"');
    if trimmed.is_empty()
        || trimmed.contains('\\')
        || trimmed.contains('/')
        || trimmed.contains(':')
    {
        return false;
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    let has_ext = std::path::Path::new(trimmed).extension().is_some();
    let exts: Vec<String> = if has_ext {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect()
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = if ext.is_empty() {
                dir.join(trimmed)
            } else {
                dir.join(format!("{trimmed}{ext}"))
            };
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
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
    bind!(on_browse, |s: &mut App| s.dialog(0));
    bind!(on_browse_working_dir, |s: &mut App| s.dialog(8));
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
    let compose_weak = ui.as_weak();
    install_keys(
        ui.window(),
        state.main.as_weak(),
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
        move |active| {
            if let Some(ui) = compose_weak.upgrade() {
                ui.set_composing(active);
            }
        },
        None,
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
    } else {
        state.apply_theme();
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

struct KeyAction<'a> {
    logical: &'a slint::winit_030::winit::keyboard::Key,
    modifiers: slint::winit_030::winit::keyboard::ModifiersState,
}

fn type_search_char<'a>(key: &'a KeyAction<'a>) -> Option<&'a str> {
    if key.modifiers.control_key() || key.modifiers.alt_key() || key.modifiers.super_key() {
        return None;
    }
    let slint::winit_030::winit::keyboard::Key::Character(text) = key.logical else {
        return None;
    };
    if text.is_empty() || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(text.as_str())
}

fn apply_common_winit_event(
    window: &slint::Window,
    event: &slint::winit_030::winit::event::WindowEvent,
    main_weak: &slint::Weak<MainWindow>,
    full_frame_pending: &mut bool,
    modifiers: &mut slint::winit_030::winit::keyboard::ModifiersState,
) -> Option<slint::winit_030::EventResult> {
    use slint::winit_030::{EventResult, winit::event::WindowEvent};
    match event {
        WindowEvent::ModifiersChanged(value) => *modifiers = value.state(),
        WindowEvent::ThemeChanged(_) => {
            let _ = main_weak.upgrade_in_event_loop(|ui| ui.invoke_native_event(21, "".into()));
            return Some(EventResult::PreventDefault);
        }
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            platform::refresh_geometry(window);
            *full_frame_pending = true;
        }
        WindowEvent::RedrawRequested => {
            windowing::prepare_tooltips(window);
            if *full_frame_pending {
                *full_frame_pending = false;
                windowing::mark_full_frame(window);
            }
        }
        _ => {}
    }
    None
}

fn apply_ime_signal(
    composition: &mut raptim::gestures::CompositionGuard,
    value: &slint::winit_030::winit::event::Ime,
) -> bool {
    use raptim::gestures::ImeSignal;
    use slint::winit_030::winit::event::Ime;
    let signal = match value {
        Ime::Preedit(_, cursor) => ImeSignal::Preedit {
            has_cursor: cursor.is_some(),
        },
        Ime::Commit(_) => ImeSignal::Commit,
        Ime::Enabled | Ime::Disabled => ImeSignal::Disabled,
    };
    composition.apply(signal)
}

fn install_keys(
    window: &slint::Window,
    main_weak: slint::Weak<MainWindow>,
    action: impl Fn(&KeyAction) -> bool + 'static,
    event_action: impl Fn(&slint::winit_030::winit::event::WindowEvent) + 'static,
    compose_sink: impl Fn(bool) + 'static,
    drag_guard: Option<slint::Weak<MainWindow>>,
) {
    use slint::winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::{ElementState, WindowEvent},
            keyboard::{Key, ModifiersState, NamedKey},
        },
    };
    let guard_drag = drag_guard.is_some();
    let mut gesture = raptim::gestures::DragGuard::default();
    let mut pointer = (0., 0.);
    let mut full_frame_pending = false;
    let mut composition = raptim::gestures::CompositionGuard::default();
    let mut modifiers = ModifiersState::empty();
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
                    if gesture.suppress_move(pointer, Instant::now()) {
                        return EventResult::PreventDefault;
                    }
                }
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } => {
                    let top = drag_guard
                        .as_ref()
                        .and_then(|w| w.upgrade())
                        .map_or(44., |ui| {
                            44. + if ui.get_dock_edge() == 2 {
                                f64::from(ui.get_dock_overhang())
                            } else {
                                0.
                            }
                        });
                    if *state == ElementState::Pressed && pointer.1 >= top {
                        gesture.press(pointer, Instant::now());
                    } else if *state == ElementState::Released && gesture.release() {
                        window.dispatch_event(slint::platform::WindowEvent::PointerExited);
                    }
                }
                WindowEvent::Focused(false) | WindowEvent::CursorLeft { .. } => {
                    gesture.reset();
                }
                _ => {}
            }
        }
        if let Some(result) = apply_common_winit_event(
            window,
            event,
            &main_weak,
            &mut full_frame_pending,
            &mut modifiers,
        ) {
            return result;
        }
        match event {
            WindowEvent::Ime(value) => {
                compose_sink(apply_ime_signal(&mut composition, value));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if composition.active() {
                    // Enter belongs to the candidate window while composing; characters must
                    // still reach the focused editor, which is the only commit-text channel.
                    return if matches!(event.logical_key, Key::Named(NamedKey::Enter)) {
                        EventResult::PreventDefault
                    } else {
                        EventResult::Propagate
                    };
                }
                if windowing::has_popup(window) {
                    return EventResult::Propagate;
                }
                if action(&KeyAction {
                    logical: &event.logical_key,
                    modifiers,
                }) {
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn install_settings_keys(ui: &SettingsWindow, main_weak: slint::Weak<MainWindow>) {
    use slint::winit_030::{
        EventResult, WinitWindowAccessor,
        winit::{
            event::{ElementState, WindowEvent},
            keyboard::{Key, NamedKey, PhysicalKey},
        },
    };
    let weak = ui.as_weak();
    let mut modifiers = slint::winit_030::winit::keyboard::ModifiersState::empty();
    let mut composition = raptim::gestures::CompositionGuard::default();
    let mut full_frame_pending = false;
    ui.window().on_winit_window_event(move |window, event| {
        if let Some(result) = apply_common_winit_event(
            window,
            event,
            &main_weak,
            &mut full_frame_pending,
            &mut modifiers,
        ) {
            return result;
        }
        match event {
            WindowEvent::Focused(active) => {
                if let Some(ui) = weak.upgrade() {
                    ui.set_window_active(*active);
                }
            }
            WindowEvent::Ime(value) => {
                let active = apply_ime_signal(&mut composition, value);
                if let Some(ui) = weak.upgrade() {
                    ui.set_composing(active);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_recording_hotkey() {
                        // A key pressed while composing belongs to the input method, not to the
                        // shortcut being recorded.
                        if composition.active() {
                            return EventResult::PreventDefault;
                        }
                        if event.logical_key == Key::Named(NamedKey::Escape) {
                            ui.invoke_captured_hotkey("".into());
                            return EventResult::PreventDefault;
                        }
                        let name = match event.physical_key {
                            PhysicalKey::Code(code) => {
                                raptim::hotkey::physical_name(&format!("{code:?}"))
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
                    if !composition.active()
                        && event.logical_key == Key::Named(NamedKey::Escape)
                        && !windowing::has_popup(window)
                    {
                        if ui.get_restore_confirm() {
                            ui.set_restore_confirm(false);
                        } else {
                            ui.invoke_cancel();
                        }
                        return EventResult::PreventDefault;
                    }
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
    let mut silent = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            data_dir = Some(PathBuf::from(args.next().ok_or("--data-dir 需要目录")?));
        } else if arg == "--portable" {
            portable = true;
        } else if arg == "--silent" {
            silent = true;
        } else {
            return Err(format!("未知参数：{}", arg.to_string_lossy()).into());
        }
    }
    if portable && data_dir.is_some() {
        return Err("--portable 与 --data-dir 不能同时使用".into());
    }
    run_at_with_mode(
        match data_dir {
            Some(path) => path,
            None => std::env::current_exe()?
                .parent()
                .ok_or("无法确定程序目录")?
                .to_path_buf(),
        },
        silent,
    )
}

fn is_permission_error(message: &str) -> bool {
    let low = message.to_lowercase();
    low.contains("access is denied")
        || low.contains("permission denied")
        || low.contains("readonly")
        || low.contains("read-only")
        || low.contains("拒绝访问")
        || low.contains("只读")
}

fn run_at_with_mode(
    data_dir: PathBuf,
    silent_startup: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Startup timeline. Each mark records ms since `run_at` began, on the same origin as every
    std::fs::create_dir_all(&data_dir)?;
    let data_dir = data_dir.canonicalize()?;
    raptim::diagnostics::attach(&data_dir);
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
        raptim::diagnostics::error("panic", &format!("{location} {payload}"));
    }));
    let Some(mut instance) = platform::Instance::acquire(&data_dir).map_err(Error::Invalid)? else {
        return Ok(());
    };
    let mut store = Store::open(&data_dir.join("raptim.db"))?;
    let initial = store.take_initial()?;
    let settings = initial.settings.clone();
    raptim::diagnostics::set_protection_log(settings.protection_log);
    let saved_window = store.window_state()?;
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::{
                platform::windows::{CornerPreference, WindowAttributesExtWindows},
                window::WindowButtons,
            };
            attributes
                .with_transparent(false)
                .with_enabled_buttons(WindowButtons::CLOSE)
                .with_skip_taskbar(true)
                .with_drag_and_drop(false)
                .with_undecorated_shadow(false)
                .with_corner_preference(CornerPreference::Round)
                .with_border_color(None)
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
        dock_overhang: None,
        edge_hidden: false,
        collapsed_far: false,
        protected_edge_waiting: false,
        dock_work: None,
        edge_progress: 0.,
        edge_motion: None,
        focus_hide: Default::default(),
        edge_hold: Default::default(),
        edge_since: Instant::now(),
        search_index: Default::default(),
        icon_cache: {
            let weak = main.as_weak();
            raptim::icons::Cache::new(move || {
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
        silent_startup,
        discovered_apps: Vec::new(),
        discovered_checked: Vec::new(),
        broken_items: Vec::new(),
    }));
    app.borrow_mut().weak_self = Rc::downgrade(&app);
    macro_rules! bind { ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => { { let weak = Rc::downgrade(&app); $ui.$callback(move |$($arg),*| { if let Some(app) = weak.upgrade() { #[allow(unused_mut)] let mut $s = app.borrow_mut(); $body } }); } }; }
    bind!(main, on_search, |s, _query| {
        s.reset_batch_selection();
        s.main.set_selected_id(0);
        s.render_search();
        s.update_visible_icons();
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
                return;
            }
            // The caller handed over `old` so a reply to a stale request cannot overwrite a newer
            // icon, which is also what the row must still hold for the band sweep to be correct.
            let usable = s
                .icon_index
                .get(&id)
                .and_then(|slot| s.entries.get(slot.index))
                .is_some_and(|e| e.target.duplicate_key() == key && e.icon == old);
            if usable && s.replace_icon(id, icon) {
                s.update_visible_icons();
                s.status("图标缓存已刷新");
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
            let mut applied = false;
            for (id, key, icon) in icons {
                let stale = s
                    .icon_index
                    .get(&id)
                    .and_then(|slot| s.entries.get(slot.index))
                    .is_none_or(|e| !e.icon.is_empty() || e.target.duplicate_key() != key);
                if stale {
                    continue;
                }
                applied |= s.replace_icon(id, icon);
            }
            if applied {
                s.update_visible_icons();
            }
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
                session.window.set_entry_icon(raptim::icons::display(&icon));
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
                            raptim::paths::convert_target(&entry.target, &s.data_dir, true)
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
            s.show_relocate(failure.entry, &failure.message, failure.relocatable);
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
        s.edge_hold.clear();
        s.sync_dock_overhang();
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
            match raptim::paths::convert_target(&entry.target, &s.data_dir, relative) {
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
        match serde_json::from_str::<raptim::importing::ExternalDrop>(&payload) {
            Ok(raptim::importing::ExternalDrop::Files(paths)) => {
                if paths.is_empty() {
                    s.status("每次最多添加 500 项");
                } else if let Err(e) = s.queue_files(paths, destination) {
                    s.status(e);
                }
            }
            Ok(raptim::importing::ExternalDrop::Text(text)) => {
                match raptim::importing::url_entries(&text, destination)
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
            if let Err(e) = s
                .store
                .move_to_collection(i64::from(id), i64::from(destination))
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
        if s.main.get_discover_open() {
            s.main.set_discover_open(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_broken_open() {
            s.main.set_broken_open(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_relocate_open() {
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
            s.dismiss_search_state();
            s.update_visible_icons();
        } else if s.main.get_selected_id() != 0 {
            s.main.set_selected_id(0);
            s.main.set_keyboard_focus(false);
        } else {
            s.hide_all();
        }
    });
    bind!(main, on_discover_apps, |s| {
        s.begin_discover_apps();
    });
    bind!(main, on_discover_refilter, |s| {
        s.render_discover_list();
    });
    bind!(main, on_discover_toggle, |s, idx| {
        s.toggle_discovered(idx);
    });
    bind!(main, on_discover_all, |s, checked| {
        s.select_all_discovered(checked);
    });
    bind!(main, on_discover_import, |s| {
        s.import_discovered();
    });
    bind!(main, on_broken_check, |s| {
        s.begin_broken_check();
    });
    bind!(main, on_broken_relocate, |s, id| {
        s.relocate_pending = i64::from(id);
        s.begin_relocate();
    });
    {
        let app = Rc::clone(&app);
        main.on_broken_edit(move |id| {
            app.borrow_mut().main.set_broken_open(false);
            open_editor(&app, i64::from(id));
        });
    }
    bind!(main, on_broken_remove, |s, id| {
        s.remove_broken_entry(i64::from(id));
    });
    bind!(main, on_native_event, |s, code, text| {
        if matches!(code, 1 | 3 | 4 | 6 | 7) && text.parse::<u64>().ok() != Some(s.policy.ticket())
        {
            return;
        }
        match code {
            1 => s.toggle_hotkey_visibility(),
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
            18 => {
                if let Ok(apps) = serde_json::from_str::<Vec<platform::DiscoveredApp>>(&text) {
                    s.discovered_checked = vec![false; apps.len()];
                    s.discovered_apps = apps;
                    s.main.set_discover_loading(false);
                    s.render_discover_list();
                    if s.main.get_discover_open() {
                        s.main.invoke_focus_discover_filter();
                    }
                }
            }
            19 => {
                if let Ok((epoch, items)) = serde_json::from_str::<(u64, Vec<BrokenEntry>)>(&text)
                    && epoch == s.data_epoch
                {
                    s.broken_items = items;
                    s.main.set_broken_loading(false);
                    s.sync_broken_ui();
                }
            }
            20 => {
                if let Ok(step) = text.parse::<i32>() {
                    s.step_category(step);
                }
            }
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
                s.edge_hold.clear();
                s.sync_dock_overhang();
                platform::restore_position(s.main.window());
                platform::decorate(s.main.window());
            }
            21 => s.apply_theme(),
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
        main.as_weak(),
        move |key| {
            use slint::winit_030::winit::keyboard::{Key, NamedKey};
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            let control = key.modifiers.control_key();
            let shift = key.modifiers.shift_key();
            let alt = key.modifiers.alt_key();
            let super_key = key.modifiers.super_key();

            if ui.get_discover_open() || ui.get_broken_open() {
                if key.logical == &Key::Named(NamedKey::Escape) {
                    ui.invoke_escape();
                    return true;
                }
                return false;
            }
            if ui.get_batch_target_open() || ui.get_batch_confirm() {
                if key.logical == &Key::Named(NamedKey::Escape) {
                    ui.invoke_escape();
                    return true;
                }
                if ui.get_batch_confirm() && key.logical == &Key::Named(NamedKey::Enter) {
                    ui.invoke_batch_execute(2, 0);
                    return true;
                }
                return control
                    && matches!(
                        key.logical,
                        Key::Character(c) if c.eq_ignore_ascii_case("a") || c.eq_ignore_ascii_case("f")
                    );
            }
            if ui.get_collection_delete()
                || ui.get_confirm_delete()
                || ui.get_relocate_open()
                || ui.get_conversion_open()
            {
                if key.logical == &Key::Named(NamedKey::Escape) {
                    ui.invoke_escape();
                    return true;
                }
                return false;
            }
            if key.logical == &Key::Named(NamedKey::Escape) {
                ui.invoke_escape();
                return true;
            }
            if control
                && !alt
                && !super_key
                && matches!(key.logical, Key::Character(c) if c.eq_ignore_ascii_case("f"))
            {
                ui.invoke_focus_search();
                return true;
            }
            if control
                && !alt
                && !super_key
                && matches!(key.logical, Key::Character(c) if c.eq_ignore_ascii_case("a"))
                && ui.get_batch_mode()
                && !ui.get_search_focused()
                && ui.get_inline_kind() == 0
                && !ui.get_more_open()
            {
                if !ui.get_batch_all_selected() {
                    ui.invoke_batch_all();
                }
                return true;
            }
            if control
                && !alt
                && !super_key
                && matches!(key.logical, Key::Character(c) if c.eq_ignore_ascii_case("n"))
                && ui.get_inline_kind() == 0
                && !ui.get_batch_mode()
                && !ui.get_search_focused()
            {
                ui.invoke_add();
                return true;
            }
            if control
                && !alt
                && !super_key
                && matches!(key.logical, Key::Character(c) if c.as_str() == ",")
                && ui.get_inline_kind() == 0
            {
                ui.invoke_settings();
                return true;
            }
            if control
                && !alt
                && !super_key
                && key.logical == &Key::Named(NamedKey::Tab)
                && ui.get_inline_kind() == 0
                && !ui.get_batch_mode()
                && !ui.get_search_open()
            {
                ui.invoke_native_event(20, if shift { "-1" } else { "1" }.into());
                return true;
            }
            if ui.get_inline_kind() != 0 {
                return false;
            }
            if ui.get_search_open() {
                if matches!(
                    key.logical,
                    Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown)
                ) {
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
                    let next = if key.logical == &Key::Named(NamedKey::ArrowDown) {
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
                if key.logical == &Key::Named(NamedKey::Enter) && ui.get_selected_id() != 0 {
                    if shift && !control && !alt {
                        ui.invoke_item_action(ui.get_selected_id(), 3);
                        return true;
                    }
                    if alt && !control && !shift {
                        ui.invoke_edit(ui.get_selected_id());
                        return true;
                    }
                    if !ui.get_search_focused() && !control && !alt && !shift {
                        ui.invoke_launch(ui.get_selected_id());
                        return true;
                    }
                }
                if !ui.get_search_focused()
                    && !control
                    && !alt
                    && !shift
                    && key.logical == &Key::Named(NamedKey::F2)
                    && ui.get_selected_id() != 0
                {
                    ui.invoke_edit(ui.get_selected_id());
                    return true;
                }
                return false;
            }

            if matches!(
                key.logical,
                Key::Named(
                    NamedKey::ArrowLeft
                        | NamedKey::ArrowRight
                        | NamedKey::ArrowUp
                        | NamedKey::ArrowDown
                        | NamedKey::Home
                        | NamedKey::End
                )
            ) && !control
                && !alt
            {
                let items = ui.get_items();
                let count = items.row_count();
                if count == 0 {
                    return false;
                }
                let cols = if ui.get_list_view() {
                    1usize
                } else {
                    (ui.get_grid_columns().max(1)) as usize
                };
                let current = (0..count).find(|i| {
                    items
                        .row_data(*i)
                        .is_some_and(|item| item.id == ui.get_selected_id())
                });
                let next = match key.logical {
                    Key::Named(NamedKey::ArrowRight) => {
                        current.map_or(0, |i| (i + 1).min(count - 1))
                    }
                    Key::Named(NamedKey::ArrowLeft) => {
                        current.map_or(count - 1, |i| i.saturating_sub(1))
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        current.map_or(0, |i| (i + cols).min(count - 1))
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        current.map_or(count - 1, |i| i.saturating_sub(cols))
                    }
                    Key::Named(NamedKey::Home) => 0,
                    _ => count - 1,
                };
                if let Some(item) = items.row_data(next) {
                    ui.set_keyboard_focus(true);
                    ui.set_selected_id(item.id);
                    ui.invoke_reveal_grid_row((next / cols) as i32);
                }
                return true;
            }

            if ui.get_batch_mode() {
                if key.logical == &Key::Named(NamedKey::Space)
                    && !control
                    && !alt
                    && ui.get_selected_id() != 0
                {
                    ui.invoke_launch(ui.get_selected_id());
                    return true;
                }
                return false;
            }

            if ui.get_selected_id() != 0 {
                if key.logical == &Key::Named(NamedKey::Enter) {
                    if shift && !control && !alt {
                        ui.invoke_item_action(ui.get_selected_id(), 3);
                        return true;
                    }
                    if alt && !control && !shift {
                        ui.invoke_edit(ui.get_selected_id());
                        return true;
                    }
                    if !control && !alt && !shift {
                        ui.invoke_launch(ui.get_selected_id());
                        return true;
                    }
                }
                if key.logical == &Key::Named(NamedKey::F2) && !control && !alt && !shift {
                    ui.invoke_edit(ui.get_selected_id());
                    return true;
                }
                if key.logical == &Key::Named(NamedKey::Delete) && !control && !alt && !shift {
                    let items = ui.get_items();
                    let name = (0..items.row_count())
                        .find_map(|i| {
                            items
                                .row_data(i)
                                .filter(|item| item.id == ui.get_selected_id())
                                .map(|item| item.name)
                        })
                        .unwrap_or_default();
                    ui.set_delete_name(name);
                    ui.set_inline_error("".into());
                    ui.set_confirm_delete(true);
                    return true;
                }
            }

            if ui.get_type_to_search()
                && !ui.get_more_open()
                && let Some(ch) = type_search_char(key)
            {
                ui.invoke_start_type_search(ch.into());
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
                    _ => {}
                }
            }
        },
        move |active| {
            if let Some(ui) = compose_weak.upgrade() {
                ui.set_composing(active);
            }
        },
        Some(main.as_weak()),
    );
    ui_commands::bind_ui_commands(&app);
    {
        let mut s = app.borrow_mut();
        s.refresh()?;
        s.sync_settings_ui();
        s.apply_theme();
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
                if s.should_poll_tick() {
                    s.evaluate();
                }
                if !s.pending_files.is_empty() {
                    s.process_files();
                }
                s.save_window(false);
            }
        },
    );
    slint::run_event_loop_until_quit()?;
    Ok(())
}
fn main() {
    // Slint and tray-icon share muda's process-global, single-assignment handler.
    // Application menus use TrackPopupMenuEx directly; widget popups stay in Slint
    // so neither path takes muda's tray handler. Set before worker threads exist.
    unsafe {
        std::env::set_var("SLINT_NO_MUDA", "1");
    }
    let message = match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return,
        Ok(Err(error)) => {
            eprintln!("Raptim: {error}");
            ("Raptim 无法启动", error.to_string())
        }
        Err(payload) => {
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "发生意外错误".into());
            raptim::diagnostics::error("fatal", &text);
            ("Raptim 运行中断", text)
        }
    };
    let detail = raptim::diagnostics::error_log_path()
        .map(|path| format!("\n详细记录：{}", path.display()))
        .unwrap_or_default();
    let permission_hint = if is_permission_error(&message.1) {
        "\n\n提示：当前数据目录可能只读或缺少写入权限，可通过 `--data-dir <可写目录>` 指定数据位置，或将程序放到用户有写入权限的文件夹。"
    } else {
        ""
    };
    rfd::MessageDialog::new()
        .set_title(message.0)
        .set_description(format!("{}{detail}{permission_hint}", message.1))
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
    main_weak: slint::Weak<MainWindow>,
    data: &std::path::Path,
) -> std::result::Result<SettingsWindow, slint::PlatformError> {
    let settings_ui = SettingsWindow::new()?;
    settings_ui.set_rust_version(env!("RAPTIM_RUST_VERSION").into());
    settings_ui.set_data_path(
        raptim::paths::normal_path(data)
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    settings_ui
        .window()
        .set_size(slint::LogicalSize::new(660., 480.));
    install_settings_keys(&settings_ui, main_weak);
    macro_rules! bind { ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => { { let weak = Rc::downgrade(app); $ui.$callback(move |$($arg),*| { if let Some(app) = weak.upgrade() { #[allow(unused_mut)] let mut $s = app.borrow_mut(); $body } }); } }; }
    settings_ui.on_open_rust(|| {
        let _ = platform::launch(&Target::Url {
            url: "https://www.rust-lang.org/".into(),
        });
    });
    settings_ui.on_open_slint(|| {
        let _ = platform::launch(&Target::Url {
            url: "https://slint.dev/".into(),
        });
    });
    bind!(settings_ui, on_record_hotkey, |s| {
        s.settings_ui
            .set_recording_hotkey(!s.settings_ui.get_recording_hotkey());
        s.settings_ui.set_hotkey_error("".into());
        let enabled = !s.settings_ui.get_recording_hotkey() && s.policy.hotkey_enabled();
        let label = s.settings.hotkey.clone();
        s.hotkey.set_enabled(&label, enabled);
        // A foreground request still goes through the policy, even from the settings window.
        if s.allowed(ShowSource::Settings) {
            platform::focus(s.settings_ui.window());
        }
    });
    bind!(settings_ui, on_reset_hotkey, |s| {
        s.settings_ui.set_recording_hotkey(false);
        s.settings_ui
            .set_hotkey_text(raptim::hotkey::DEFAULT.into());
        s.apply_settings(2);
        s.evaluate();
    });
    bind!(settings_ui, on_captured_hotkey, |s, value| {
        s.settings_ui.set_recording_hotkey(false);
        if !value.is_empty() {
            match raptim::hotkey::parse(&value) {
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
    bind!(settings_ui, on_import_data, |s| {
        s.dialog(4);
    });
    bind!(settings_ui, on_confirm_restore, |s| {
        if let Err(e) = s.restore() {
            s.settings_ui.set_status(e.to_string().into());
        }
    });
    bind!(settings_ui, on_restore_position, |s| {
        s.show_main(ShowSource::Restore);
        s.dock = None;
        s.edge_hold.clear();
        s.sync_dock_overhang();
        platform::restore_position(s.main.window());
        platform::decorate(s.main.window());
        s.save_window(true);
    });
    bind!(settings_ui, on_open_data_dir, |s| {
        if let Err(e) = platform::launch(&Target::Folder {
            path: s.data_dir.to_string_lossy().into_owned(),
        }) {
            s.settings_ui.set_status(e.message().into());
        }
    });
    bind!(settings_ui, on_open_error_log, |s| {
        if let Some(path) = raptim::diagnostics::error_log_path()
            && path.exists()
        {
            if let Err(e) = platform::reveal_file(&path) {
                s.settings_ui.set_status(e.into());
            }
        } else {
            s.settings_ui.set_has_error_log(false);
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

#[cfg(test)]
mod unit_tests {
    use super::*;
    use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};

    #[test]
    fn type_search_accepts_printable_keys_and_skips_modifiers_or_whitespace() {
        let plain = ModifiersState::empty();
        let shift = ModifiersState::SHIFT;
        let ctrl = ModifiersState::CONTROL;

        let g = Key::Character("g".into());
        let plus = Key::Character("+".into());
        let space = Key::Character(" ".into());
        let tab = Key::Named(NamedKey::Tab);

        assert_eq!(
            type_search_char(&KeyAction {
                logical: &g,
                modifiers: plain
            }),
            Some("g")
        );
        assert_eq!(
            type_search_char(&KeyAction {
                logical: &plus,
                modifiers: shift
            }),
            Some("+")
        );
        assert_eq!(
            type_search_char(&KeyAction {
                logical: &g,
                modifiers: ctrl
            }),
            None
        );
        assert_eq!(
            type_search_char(&KeyAction {
                logical: &space,
                modifiers: plain
            }),
            None
        );
        assert_eq!(
            type_search_char(&KeyAction {
                logical: &tab,
                modifiers: plain
            }),
            None
        );
    }

    #[test]
    fn detects_permission_errors_and_bare_commands_on_path() {
        assert!(is_permission_error("attempt to write a readonly database"));
        assert!(is_permission_error("拒绝访问"));
        assert!(!is_permission_error("invalid input"));
        assert!(command_exists("cmd"));
        assert!(!command_exists("C:\\Windows\\System32\\cmd.exe"));
        assert!(!command_exists("raptim_definitely_missing_binary_xyz"));
    }
}
