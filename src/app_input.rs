// Winit window event hooks, IME composition guard, and keyboard navigation bindings.
use super::*;

pub(super) struct KeyAction<'a> {
    pub(super) logical: &'a slint::winit_030::winit::keyboard::Key,
    pub(super) modifiers: slint::winit_030::winit::keyboard::ModifiersState,
}

pub(super) fn type_search_char<'a>(key: &'a KeyAction<'a>) -> Option<&'a str> {
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
    events: &EventSink,
    full_frame_pending: &mut bool,
    modifiers: &mut slint::winit_030::winit::keyboard::ModifiersState,
) -> Option<slint::winit_030::EventResult> {
    use slint::winit_030::{EventResult, winit::event::WindowEvent};
    match event {
        WindowEvent::ModifiersChanged(value) => *modifiers = value.state(),
        WindowEvent::ThemeChanged(_) => {
            events.send(AppEvent::ThemeChanged);
            return Some(EventResult::PreventDefault);
        }
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            platform::refresh_geometry(window);
            *full_frame_pending = true;
        }
        WindowEvent::Occluded(false) => {
            *full_frame_pending = true;
        }
        WindowEvent::RedrawRequested => {
            windowing::prepare_tooltips(window);
            if *full_frame_pending {
                if platform::surface_paintable(window) {
                    *full_frame_pending = false;
                    let _ = platform::take_os_dirty(window);
                    windowing::mark_full_frame(window);
                }
            } else if let Some(dirty) = platform::take_os_dirty(window) {
                match dirty {
                    platform::OsDirty::Full => windowing::mark_full_frame(window),
                    platform::OsDirty::Rect {
                        left,
                        top,
                        right,
                        bottom,
                    } => windowing::mark_physical_rect(window, left, top, right, bottom),
                }
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

pub(super) fn install_keys(
    window: &slint::Window,
    events: EventSink,
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
    // Force a full repaint on the first RedrawRequested after register_window; Slint's
    // ShownFirstTime pre-renders before ShowWindow, which DWM discards on Windows.
    let mut full_frame_pending = true;
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
            &events,
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

pub(super) fn install_settings_keys(ui: &SettingsWindow, events: EventSink) {
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
    let mut full_frame_pending = true;
    ui.window().on_winit_window_event(move |window, event| {
        if let Some(result) = apply_common_winit_event(
            window,
            event,
            &events,
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
                        if ui.get_legal_open() {
                            ui.set_legal_open(false);
                        } else if ui.get_restore_confirm() {
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

pub(super) fn bind_main_input(main: &MainWindow, events: EventSink) {
    let weak = main.as_weak();
    let step_events = events.clone();
    let window_events = events.clone();
    let compose_weak = main.as_weak();
    install_keys(
        main.window(),
        events,
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
                step_events.send(AppEvent::StepCategory(if shift { -1 } else { 1 }));
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
                if !control
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
                        current.map_or(count - 1, |i| if i >= cols { i - cols } else { i })
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
            match event {
                WindowEvent::Moved(_) => window_events.send(AppEvent::WindowMoved),
                WindowEvent::Focused(_)
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. } => {
                    window_events.send(AppEvent::WindowInteraction);
                }
                _ => {}
            }
        },
        move |active| {
            if let Some(ui) = compose_weak.upgrade() {
                ui.set_composing(active);
            }
        },
        Some(main.as_weak()),
    );
}
