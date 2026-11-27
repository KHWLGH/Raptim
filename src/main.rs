#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use raptim::{
    AppOption, BrokenOption, DestinationNode, DiscoverOption, EditorWindow, I18n, Item, MainWindow,
    NavItem, NoticeItem, Palette, ScopeNode, SettingsWindow,
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

mod app_editor;
mod app_events;
mod app_input;
mod app_session;
mod app_settings;
mod app_tools;
mod ui_commands;

use app_editor::{EditorSession, open_editor};
use app_events::{AppEvent, DialogKind, EventSink};
use app_input::{bind_main_input, install_keys, install_settings_keys};
use app_settings::Deferred;
use app_tools::BrokenEntry;

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
    events: EventSink,
    events_rx: std::sync::mpsc::Receiver<AppEvent>,
    protection_resume: raptim::session::ProtectionResume,
    editor_generation: u64,
    batch_expanded: std::collections::HashSet<i64>,
    editor_expanded: std::collections::HashSet<i64>,
    external_drop: Option<platform::drop_target::Registration>,
    main: MainWindow,
    weak_self: std::rc::Weak<RefCell<App>>,
    interaction_timer: slint::Timer,
    poll_timer: slint::Timer,
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

    fn child_topmost(&self) -> bool {
        platform::is_topmost(self.main.window()) || self.wants_topmost()
    }

    fn apply_main_topmost(&self, enabled: bool) {
        platform::topmost(self.main.window(), enabled);
        self.sync_child_layering();
    }

    fn sync_child_layering(&self) {
        let main_topmost = platform::is_topmost(self.main.window());
        if let Some(ui) = self.settings_ui.get()
            && ui.window().is_visible()
        {
            ui.set_topmost_window(main_topmost);
            platform::set_owner(ui.window(), self.main.window());
        }
        if let Some(editor) = &self.editor
            && editor.window.window().is_visible()
        {
            editor.window.set_topmost_window(main_topmost);
            platform::set_owner(editor.window.window(), self.main.window());
        }
    }

    fn show_child_window(
        &self,
        child: &slint::Window,
    ) -> std::result::Result<(), slint::PlatformError> {
        let weak = self.weak_self.clone();
        let sink = self.events.clone();
        windowing::show_child_with(child, self.main.window(), move || {
            if let Some(app) = weak.upgrade()
                && let Ok(s) = app.try_borrow()
            {
                s.apply_theme();
            } else {
                sink.send(AppEvent::ThemeChanged);
            }
        })
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

    fn apply_language(&self) {
        let en =
            raptim::i18n::apply_language_mode(self.settings.language) == raptim::i18n::Locale::EnUs;
        self.main.global::<I18n>().set_en(en);
        if let Some(ui) = self.settings_ui.get() {
            ui.global::<I18n>().set_en(en);
            ui.set_notice_items(app_settings::notice_items());
        }
        if let Some(editor) = &self.editor {
            editor.window.global::<I18n>().set_en(en);
        }
        self.tray.refresh_language();
        windowing::invalidate(self.main.window());
        if let Some(ui) = self.settings_ui.get() {
            windowing::invalidate(ui.window());
        }
        if let Some(editor) = &self.editor {
            windowing::invalidate(editor.window.window());
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
            self.apply_main_topmost(self.wants_topmost());
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
        self.apply_main_topmost(self.wants_topmost());
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
            raptim::i18n::t(
                "已暂停游戏勿扰，仅本次运行有效",
                "Game DND paused for this session",
            )
        } else {
            raptim::i18n::t("已恢复游戏勿扰判定", "Game DND resumed")
        });
    }

    fn animate_edge(&mut self, hidden: bool) {
        if self.policy.protection() != Protection::Normal {
            self.settle_edge_hidden();
            return;
        }
        if hidden {
            self.edge_hold.clear();
        } else {
            self.focus_hide.reset();
            platform::set_opacity(self.main.window(), 1.);
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
        self.apply_main_topmost(true);
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
        if self.main.get_search_open() != self.search_was_open {
            if self.main.get_search_open() {
                self.main.set_selected_id(0);
            } else {
                self.main.invoke_restore_grid_focus();
            }
            self.search_was_open = self.main.get_search_open();
            self.render_search();
            self.update_visible_icons();
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
        let captured = platform::has_mouse_capture(self.main.window());
        let paused = (!self.protected_edge_waiting && self.auto_hide_guard().paused())
            || (platform::mouse_down()
                && ((captured && !self.protected_edge_waiting && self.edge_progress < 0.5)
                    || (self.edge_progress == 0.
                        && platform::geometry(self.main.window())
                            .is_some_and(|(rect, _)| raptim::docking::contains(rect, cursor)))));
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
                        self.apply_main_topmost(self.wants_topmost());
                    }
                }
                self.sync_edge_appearance();
                if done && !self.edge_hidden {
                    windowing::invalidate(self.main.window());
                } else if self.edge_progress > 0.60 || done {
                    platform::request_redraw_now(self.main.window());
                }
            }
        } else {
            self.edge_hold.clear();
            let focused = platform::app_has_focus();
            let animated = if self.settings.hide_on_focus_loss && !focused {
                platform::animations_enabled()
            } else {
                false
            };
            let opacity =
                self.focus_hide
                    .opacity(self.settings.hide_on_focus_loss, focused, animated, now);
            platform::set_opacity(self.main.window(), opacity);
            if opacity == 0. && self.settings.hide_on_focus_loss {
                self.hide_all();
                self.focus_hide.reset();
            }
        }
    }

    /// Cursor distance from the reveal band beyond which the collapsed poll drops to half rate.
    const APPROACH_MARGIN_PX: i32 = 96;

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
            return false;
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

    /// Fill the icons of entries that have none. Every lookup blocks — a Shell call or a favicon
    /// fetch — so the work goes to a background thread and lands in batches.
    fn fill_icons(&self, wanted: Vec<(i64, Target)>) {
        let wanted = wanted
            .into_iter()
            .map(|(id, target)| (id, target, None))
            .collect();
        self.fill_icons_with_sources(wanted);
    }

    fn fill_icons_with_sources(&self, wanted: Vec<(i64, Target, Option<PathBuf>)>) {
        if wanted.is_empty() {
            return;
        }
        let base = self.data_dir.clone();
        let epoch = self.data_epoch;
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let mut batch = Vec::new();
            for (id, target, source) in wanted {
                let icon = source
                    .as_deref()
                    .map(platform::shell_icon)
                    .filter(|bytes| !bytes.is_empty())
                    .unwrap_or_else(|| platform::icon_for(&target, &base));
                if !icon.is_empty() {
                    batch.push((id, target.duplicate_key(), icon));
                }
                if batch.len() >= 16 {
                    sink.send(AppEvent::BatchIconsReady {
                        epoch,
                        icons: std::mem::take(&mut batch),
                    });
                }
            }
            if !batch.is_empty() {
                sink.send(AppEvent::BatchIconsReady {
                    epoch,
                    icons: batch,
                });
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
        let rehashed: std::collections::HashSet<i64> = if let Some(initial) = self.initial.take() {
            self.collections = initial.collections;
            self.entries = initial.entries;
            self.entries.iter().map(|e| e.id).collect()
        } else {
            self.collections = self.store.collections()?;
            let (entries, rehashed) = self.store.entries_reusing_icons(&mut self.entries)?;
            self.entries = entries;
            rehashed
        };
        self.search_index.sync(&self.entries);
        let ids: std::collections::HashSet<_> = self.entries.iter().map(|e| e.id).collect();
        self.icon_cache.retain(&ids);
        let prev_index = std::mem::take(&mut self.icon_index);
        self.icon_index = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, e)| {
                let fingerprint = if !rehashed.contains(&e.id)
                    && let Some(prev) = prev_index.get(&e.id)
                {
                    prev.fingerprint
                } else {
                    raptim::icons::fingerprint(&e.icon)
                };
                (e.id, raptim::icons::IconKey { fingerprint, index })
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
        if self.main.get_search_open() {
            self.render_search();
        }
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
            self.restore_protected_edge();
            let deferred = self.notices.borrow_mut().take();
            if !deferred.is_empty() {
                self.main.set_status(deferred.into());
            }
        }
        self.sync_poll_timer();
    }

    fn allowed(&mut self, source: ShowSource) -> bool {
        let ticket = self.policy.ticket();
        self.evaluate();
        let allowed = self.policy.allows(source, ticket);
        if !allowed {
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
        self.update_visible_icons();
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
            ui.set_topmost_window(false);
            platform::topmost(ui.window(), false);
        }
        if let Some(editor) = &self.editor {
            windowing::close_popups(editor.window.window());
            let _ = editor.window.hide();
            editor.window.set_topmost_window(false);
            platform::topmost(editor.window.window(), false);
        }
        // Keep the current viewport band hot in cache so re-summoning paints complete icons on the
        // very first frame. If a preceding search update cleared `visible_icons`, repopulate it first.
        if self.visible_icons.is_empty() {
            self.update_visible_icons();
        }
        self.icon_cache.retain(&self.visible_icons);
        self.main.invoke_close_popups();
        self.sync_poll_timer();
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
                    self.sync_poll_timer();
                }
            }
            return;
        }
        let visible_edge = self.main.window().is_visible()
            && self.dock.is_some()
            && (self.edge_progress > 0. || self.edge_motion.is_some());
        if !self.main.window().is_visible()
            || (platform::native_ready(self.main.window())
                && !platform::is_native_visible(self.main.window()))
        {
            if platform::native_ready(self.main.window()) {
                self.apply_main_topmost(self.wants_topmost());
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
            windowing::invalidate(self.main.window());
        }
        self.schedule_interaction();
        platform::launcher_window(self.main.window());
        self.apply_main_topmost(self.edge_motion.is_some() || self.wants_topmost());
        platform::focus(self.main.window());
        self.focus_hide.reset();
        if self.dock.is_some() && self.settings.edge_hide && !self.edge_hidden {
            self.edge_hold
                .arm(Instant::now(), platform::app_has_focus());
        }
        if self.settings_open {
            self.settings_ui.set_topmost_window(self.child_topmost());
            let _ = self.show_child_window(self.settings_ui.window());
        }
        if let Some(editor) = &self.editor {
            editor.window.set_topmost_window(self.child_topmost());
            let _ = self.show_child_window(editor.window.window());
        }
        self.apply_theme();
        self.sync_poll_timer();
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
                    self.status(raptim::i18n::t(
                        "该类型不支持管理员打开",
                        "This target type does not support running as administrator",
                    ));
                    return;
                }
            };
        }
        self.main.set_busy(true);
        self.main.set_status("".into());
        let sink = self.events.clone();
        let settings = self.settings.clone();
        let bypass = self.policy.bypass();
        std::thread::spawn(move || {
            let failure = if bypass || platform::protection(&settings) == Protection::Normal {
                platform::launch(&target).err().map(|error| Failure {
                    entry: id,
                    message: error.message(),
                    relocatable: error.relocatable(),
                })
            } else {
                Some(Failure {
                    entry: id,
                    message: raptim::i18n::t(
                        "当前处于全屏或游戏保护，已取消启动",
                        "Launch canceled: fullscreen or Game DND protection active",
                    )
                    .into(),
                    relocatable: false,
                })
            };
            sink.send(AppEvent::LaunchFinished(failure));
        });
    }

    fn request_quit(&mut self) {
        self.protection_resume.cancel();
        self.save_window(true);
        if self.importing || !self.pending_files.is_empty() {
            self.status(raptim::i18n::t(
                "正在收录文件，请完成后再退出",
                "Importing files; wait until finished before exiting",
            ));
            return;
        }
        if self.file_dialog_open {
            self.status(raptim::i18n::t(
                "请先关闭文件选择窗口",
                "Please close the file dialog first",
            ));
            return;
        }
        self.quitting = true;
        if self.editor_dirty() {
            if self.allowed(ShowSource::Editor) {
                let topmost = self.child_topmost();
                let editor = self.editor.as_ref().unwrap();
                editor.window.set_topmost_window(topmost);
                editor.window.set_discard_confirm(true);
                let _ = self.show_child_window(editor.window.window());
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

    fn dialog(&mut self, kind: DialogKind) {
        if self.file_dialog_open || !self.allowed(ShowSource::Settings) {
            return;
        }
        let folder = match kind {
            DialogKind::EditorTarget => self
                .editor
                .as_ref()
                .is_some_and(|s| s.window.get_kind() == 2),
            DialogKind::RelocateTarget => self.entries.iter().any(|e| {
                e.id == self.relocate_pending && matches!(e.target, Target::Folder { .. })
            }),
            _ => false,
        };
        self.file_dialog_open = true;
        self.user_summon_edge();
        self.dialog_collection = self.collection;
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new();
            if kind == DialogKind::AddFiles {
                let paths = dialog.pick_files().unwrap_or_default();
                sink.send(AppEvent::FilesPicked(paths));
                return;
            }
            let path = match kind {
                DialogKind::EditorTarget if folder => dialog.pick_folder(),
                DialogKind::EditorTarget => dialog.pick_file(),
                DialogKind::EditorIcon => dialog
                    .add_filter(
                        raptim::i18n::t("图标", "Images"),
                        &["png", "jpg", "jpeg", "ico"],
                    )
                    .pick_file(),
                DialogKind::ProtectedApp => dialog
                    .add_filter(raptim::i18n::t("程序", "Programs"), &["exe"])
                    .pick_file(),
                DialogKind::ExportJson => dialog
                    .add_filter(raptim::i18n::t("Raptim 导出", "Raptim Export"), &["json"])
                    .set_file_name("raptim-export.json")
                    .save_file(),
                DialogKind::RelocateTarget if folder => dialog.pick_folder(),
                DialogKind::RelocateTarget => dialog.pick_file(),
                DialogKind::EditorWorkingDir => dialog.pick_folder(),
                DialogKind::RestoreJson => dialog
                    .add_filter(raptim::i18n::t("Raptim 导出", "Raptim Export"), &["json"])
                    .pick_file(),
                DialogKind::AddFiles => unreachable!(),
            };
            let value = path
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            sink.send(AppEvent::FileSelected { kind, value });
        });
    }

    fn file_selected(&mut self, kind: DialogKind, value: String) -> Result<()> {
        self.file_dialog_open = false;
        self.focus_hide.reset();
        self.edge_since = Instant::now();
        if value.is_empty() {
            return Ok(());
        }
        let path = PathBuf::from(&value);
        match kind {
            DialogKind::EditorIcon => {
                let icon = raptim::icons::from_file(&path)?;
                self.icon_epoch += 1;
                self.icon_pending = false;
                if let Some(session) = &mut self.editor {
                    session.window.set_entry_icon(raptim::icons::display(&icon));
                    session.icon = icon;
                }
            }
            DialogKind::AddFiles => unreachable!(),
            DialogKind::EditorTarget => {
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
                self.extract_editor_icon();
            }
            DialogKind::EditorWorkingDir => {
                if let Some(session) = &self.editor {
                    session.window.set_working_dir(value.into());
                }
            }
            DialogKind::ProtectedApp => {
                self.add_protected_app(value);
            }
            DialogKind::ExportJson => {
                use std::io::Write;
                let bytes = serde_json::to_vec_pretty(&self.store.export()?)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                self.settings_ui
                    .set_status(raptim::i18n::t("已导出", "Exported").into());
            }
            DialogKind::RelocateTarget => self.apply_relocate(value)?,
            DialogKind::RestoreJson => {
                if self.editor.is_some() {
                    return Err(Error::Invalid(
                        raptim::i18n::t(
                            "请先保存或取消项目编辑",
                            "Save or discard the current item edit first",
                        )
                        .into(),
                    ));
                }
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(32 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(Error::Invalid(
                        raptim::i18n::t("导出文件超过 32 MB", "Export file exceeds 32 MB").into(),
                    ));
                }
                let data: Export = serde_json::from_slice(&bytes)?;
                raptim::storage::validate_export(&data)?;
                if let Some(name) = self.store.export_clash(&data.entries) {
                    return Err(Error::Invalid(if raptim::i18n::is_en() {
                        format!(
                            "Different representations point to the same target in one category: {name}; no changes made"
                        )
                    } else {
                        format!("同一分类内有指向同一目标的不同写法：{name}；未改动数据")
                    }));
                }
                self.settings_ui.set_status(
                    if raptim::i18n::is_en() {
                        format!(
                            "Will restore {} collection(s) and {} item(s)",
                            data.collections.len(),
                            data.entries.len()
                        )
                    } else {
                        format!(
                            "将恢复 {} 个分组/分类、{} 个项目",
                            data.collections.len(),
                            data.entries.len()
                        )
                    }
                    .into(),
                );
                self.pending_import = Some(data);
                self.settings_ui.set_restore_confirm(true);
            }
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
        self.sync_poll_timer();
        if overflow {
            return Err(Error::Invalid(
                raptim::i18n::t(
                    "每次最多添加 500 项，请分批拖入",
                    "You can add at most 500 items at once; drop in smaller batches",
                )
                .into(),
            ));
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
        self.status(raptim::i18n::t("正在读取文件…", "Reading files…"));
        let paths = std::mem::take(&mut self.pending_files);
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let result = raptim::importing::file_entries(paths).map_err(|e| e.to_string());
            sink.send(AppEvent::FilesReady(result));
        });
    }

    fn restore(&mut self) -> Result<()> {
        if self.importing || !self.pending_files.is_empty() || self.main.get_busy() {
            return Err(Error::Invalid(
                raptim::i18n::t(
                    "请先等待文件收录完成，再恢复数据",
                    "Wait for file import to finish before restoring data",
                )
                .into(),
            ));
        }
        if self.editor.is_some() {
            return Err(Error::Invalid(
                raptim::i18n::t(
                    "请先保存或取消项目编辑",
                    "Save or discard the current item edit first",
                )
                .into(),
            ));
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
        self.apply_language();
        self.sync_settings_ui();
        self.apply_theme();
        self.refresh()?;
        self.pending_import = None;
        self.settings_ui.set_restore_confirm(false);
        self.settings_ui
            .set_status(raptim::i18n::t("已恢复", "Restored").into());
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

    fn needs_poll_timer(&self) -> bool {
        self.should_poll_tick() || !self.pending_files.is_empty() || self.window_candidate.is_some()
    }

    fn sync_poll_timer(&self) {
        if self.needs_poll_timer() {
            if self.poll_timer.running() {
                return;
            }
            let weak = self.weak_self.clone();
            self.poll_timer.start(
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
                        s.sync_poll_timer();
                    }
                },
            );
        } else if self.poll_timer.running() {
            self.poll_timer.stop();
        }
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
}

fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    raptim::i18n::apply_language_mode(raptim::i18n::LANGUAGE_SYSTEM);
    let mut data_dir = None;
    let mut portable = false;
    let mut silent = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            data_dir = Some(PathBuf::from(args.next().ok_or(raptim::i18n::t(
                "--data-dir 需要目录",
                "--data-dir requires a directory",
            ))?));
        } else if arg == "--portable" {
            portable = true;
        } else if arg == "--silent" {
            silent = true;
        } else {
            return Err(format!(
                "{}: {}",
                raptim::i18n::t("未知参数", "Unknown argument"),
                arg.to_string_lossy()
            )
            .into());
        }
    }
    if portable && data_dir.is_some() {
        return Err(raptim::i18n::t(
            "--portable 与 --data-dir 不能同时使用",
            "--portable and --data-dir cannot be used together",
        )
        .into());
    }
    run_at_with_mode(
        match data_dir {
            Some(path) => path,
            None => std::env::current_exe()?
                .parent()
                .ok_or(raptim::i18n::t(
                    "无法确定程序目录",
                    "Unable to determine application directory",
                ))?
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
    std::fs::create_dir_all(&data_dir)?;
    let data_dir = data_dir.canonicalize()?;
    raptim::diagnostics::attach(&data_dir);
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map_or_else(
            || raptim::i18n::t("<未知位置>", "<unknown location>").into(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| raptim::i18n::t("<非字符串 panic>", "<non-string panic>").into());
        eprintln!("panic at {location}: {payload}");
        raptim::diagnostics::error("panic", &format!("{location} {payload}"));
    }));
    let Some(mut instance) = platform::Instance::acquire(&data_dir).map_err(Error::Invalid)? else {
        return Ok(());
    };
    let mut store = Store::open(&data_dir.join("raptim.db"))?;
    store.ensure_categories()?;
    let initial = store.take_initial()?;
    let settings = initial.settings.clone();
    raptim::i18n::apply_language_mode(settings.language);
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
    main.global::<I18n>().set_en(raptim::i18n::is_en());
    let mut policy = DisplayPolicy::default();
    let signals = platform::collect_signals(&settings, &mut platform::ProtectionCache::default());
    policy.update(decide(signals), Instant::now(), signals.hold_release());
    let native_epoch = Arc::new(AtomicU64::new(policy.ticket()));
    let (events_tx, events_rx) = std::sync::mpsc::channel();
    let events = EventSink::new(events_tx, main.as_weak());
    let epoch = native_epoch.clone();
    let notify_sink = events.clone();
    let notify: platform::Notify = Arc::new(move |signal| {
        let ticket = epoch.load(Ordering::SeqCst);
        notify_sink.send(AppEvent::Native(signal, ticket));
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
    let cache_sink = events.clone();
    let app = Rc::new(RefCell::new(App {
        events: events.clone(),
        events_rx,
        protection_resume: Default::default(),
        editor_generation: 0,
        batch_expanded: Default::default(),
        editor_expanded: Default::default(),
        external_drop: None,
        main: main.clone_strong(),
        weak_self: Default::default(),
        interaction_timer: Default::default(),
        poll_timer: Default::default(),
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
        icon_cache: raptim::icons::Cache::new(move || {
            cache_sink.send(AppEvent::ViewportChanged);
        }),
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
    {
        let weak = Rc::downgrade(&app);
        main.on_flush_events(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            loop {
                let next = app
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.events_rx.try_recv().ok());
                let Some(event) = next else {
                    break;
                };
                if let Ok(mut s) = app.try_borrow_mut() {
                    s.handle_event(event);
                }
            }
        });
    }
    macro_rules! bind {
        ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => {{
            let weak = Rc::downgrade(&app);
            $ui.$callback(move |$($arg),*| {
                if let Some(app) = weak.upgrade() {
                    #[allow(unused_mut)]
                    let mut $s = app.borrow_mut();
                    $body
                }
            });
        }};
    }
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
        s.dialog(DialogKind::AddFiles);
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
            s.status(raptim::i18n::t(
                "请先关闭此项目的编辑窗口",
                "Close the editor window for this item first",
            ));
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
        s.schedule_interaction();
        platform::begin_launcher_drag(s.main.window());
    });
    bind!(main, on_convert_path, |s, id, relative| {
        if s.editor.is_some() {
            s.status(raptim::i18n::t("请先完成当前编辑", "Finish editing first"));
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
    bind!(main, on_settings, |s| {
        s.show_settings();
    });
    let interaction_sink = events.clone();
    main.on_interaction_changed(move || {
        let sink = interaction_sink.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            sink.send(AppEvent::WindowInteraction);
        });
    });
    let viewport_pending = Rc::new(std::cell::Cell::new(false));
    let viewport_sink = events.clone();
    let viewport_weak = Rc::downgrade(&app);
    main.on_viewport_changed(move || {
        if viewport_pending.replace(true) {
            return;
        }
        let pending = viewport_pending.clone();
        let sink = viewport_sink.clone();
        let weak = viewport_weak.clone();
        slint::Timer::single_shot(Duration::ZERO, move || {
            pending.set(false);
            if let Some(app) = weak.upgrade()
                && let Ok(mut s) = app.try_borrow_mut()
            {
                s.update_visible_icons();
            } else {
                sink.send(AppEvent::ViewportChanged);
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
            s.status(raptim::i18n::t(
                "请先完成此项目的编辑",
                "Finish editing this item first",
            ));
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
            s.main.invoke_focus_browser();
        } else if s.main.get_collection_delete() {
            s.main.set_collection_delete(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_confirm_delete() {
            s.main.set_confirm_delete(false);
            s.main.invoke_focus_browser();
        } else if s.main.get_batch_mode() {
            s.clear_batch();
            s.update_selection();
        } else if s.main.get_search_open() {
            s.dismiss_search_state();
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
        let weak = Rc::downgrade(&app);
        main.on_broken_edit(move |id| {
            if let Some(app) = weak.upgrade() {
                app.borrow_mut().main.set_broken_open(false);
                open_editor(&app, i64::from(id));
            }
        });
    }
    bind!(main, on_broken_remove, |s, id| {
        s.remove_broken_entry(i64::from(id));
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
    bind_main_input(&main, events);
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
        let wanted: Vec<_> = s
            .entries
            .iter()
            .filter(|e| e.icon.is_empty() && !matches!(e.target, Target::Url { .. }))
            .map(|e| (e.id, e.target.clone()))
            .collect();
        s.fill_icons(wanted);
    }

    app.borrow().sync_poll_timer();
    slint::run_event_loop_until_quit()?;
    Ok(())
}

fn main() {
    unsafe {
        std::env::set_var("SLINT_NO_MUDA", "1");
    }
    let message = match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return,
        Ok(Err(error)) => {
            eprintln!("Raptim: {error}");
            (
                raptim::i18n::t("Raptim 无法启动", "Raptim could not start"),
                error.to_string(),
            )
        }
        Err(payload) => {
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| {
                    raptim::i18n::t("发生意外错误", "An unexpected error occurred").into()
                });
            raptim::diagnostics::error("fatal", &text);
            (
                raptim::i18n::t("Raptim 运行中断", "Raptim stopped unexpectedly"),
                text,
            )
        }
    };
    let detail = raptim::diagnostics::error_log_path()
        .map(|path| {
            format!(
                "\n{}: {}",
                raptim::i18n::t("详细记录", "Error log"),
                path.display()
            )
        })
        .unwrap_or_default();
    let permission_hint = if is_permission_error(&message.1) {
        raptim::i18n::t(
            "\n\n提示：当前数据目录可能只读或缺少写入权限，可通过 `--data-dir <可写目录>` 指定数据位置，或将程序放到用户有写入权限的文件夹。",
            "\n\nTip: The data folder may be read-only or lack write permissions. Pass `--data-dir <writable-dir>` or move Raptim to a writable folder.",
        )
    } else {
        ""
    };
    rfd::MessageDialog::new()
        .set_title(message.0)
        .set_description(format!("{}{detail}{permission_hint}", message.1))
        .set_level(rfd::MessageLevel::Error)
        .show();
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use app_input::{KeyAction, type_search_char};
    use app_tools::command_exists;
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
