//! System tray icon and native context menu integration.
use super::*;

pub struct Tray {
    _icon: tray_icon::TrayIcon,
    show: tray_icon::menu::MenuItem,
    restore: tray_icon::menu::MenuItem,
    pause_hide: tray_icon::menu::CheckMenuItem,
    pause_protection: tray_icon::menu::CheckMenuItem,
    settings: tray_icon::menu::MenuItem,
    exit: tray_icon::menu::MenuItem,
}

impl Tray {
    pub fn new(notify: Notify) -> Result<Self, String> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
        };
        let menu = Menu::new();
        let show = MenuItem::new(crate::i18n::t("显示 Raptim", "Show Raptim"), true, None);
        let restore = MenuItem::new(
            crate::i18n::t("恢复窗口位置", "Restore Window Position"),
            true,
            None,
        );
        let pause_hide = CheckMenuItem::new(
            crate::i18n::t("暂停自动隐藏", "Pause Auto-Hide"),
            true,
            false,
            None,
        );
        let pause_protection = CheckMenuItem::new(
            crate::i18n::t("暂停游戏勿扰", "Pause Game DND"),
            true,
            false,
            None,
        );
        let settings = MenuItem::new(crate::i18n::t("设置", "Settings"), true, None);
        let exit = MenuItem::new(crate::i18n::t("退出", "Quit"), true, None);
        // Window · toggles · app · quit: the same four sections as the launcher's 更多 menu.
        let (sep_window, sep_toggles, sep_app) = (
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::separator(),
        );
        menu.append_items(&[
            &show,
            &restore,
            &sep_window,
            &pause_hide,
            &pause_protection,
            &sep_toggles,
            &settings,
            &sep_app,
            &exit,
        ])
        .map_err(|e| e.to_string())?;
        let ids = (
            show.id().clone(),
            restore.id().clone(),
            settings.id().clone(),
            exit.id().clone(),
            pause_hide.id().clone(),
            pause_protection.id().clone(),
        );
        let menu_notify = notify.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let signal = if event.id == ids.0 {
                NativeSignal::TrayShow
            } else if event.id == ids.1 {
                NativeSignal::TrayRestorePosition
            } else if event.id == ids.2 {
                NativeSignal::TraySettings
            } else if event.id == ids.3 {
                NativeSignal::TrayQuit
            } else if event.id == ids.4 {
                NativeSignal::TrayTogglePin
            } else if event.id == ids.5 {
                NativeSignal::TrayToggleProtectionBypass
            } else {
                return;
            };
            menu_notify(signal);
        }));
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                notify(NativeSignal::TrayShow);
            }
        }));
        let tray_rgba = image::load_from_memory(include_bytes!("../../ui/icons/raptim-tray.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8();
        let (width, height) = tray_rgba.dimensions();
        let icon =
            Icon::from_rgba(tray_rgba.into_raw(), width, height).map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("Raptim")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _icon: icon,
            show,
            restore,
            pause_hide,
            pause_protection,
            settings,
            exit,
        })
    }

    pub fn refresh_language(&self) {
        self.show
            .set_text(crate::i18n::t("显示 Raptim", "Show Raptim"));
        self.restore
            .set_text(crate::i18n::t("恢复窗口位置", "Restore Window Position"));
        self.pause_hide
            .set_text(crate::i18n::t("暂停自动隐藏", "Pause Auto-Hide"));
        self.pause_protection
            .set_text(crate::i18n::t("暂停游戏勿扰", "Pause Game DND"));
        self.settings.set_text(crate::i18n::t("设置", "Settings"));
        self.exit.set_text(crate::i18n::t("退出", "Quit"));
    }

    /// Keeps the two check marks honest after a toggle from any other entry point.
    pub fn set_states(&self, paused: bool, protection_bypassed: bool) {
        self.pause_hide.set_checked(paused);
        self.pause_protection.set_checked(protection_bypassed);
    }

    pub fn set_tooltip(&self, text: &str) {
        let _ = self._icon.set_tooltip(Some(text));
    }
}
