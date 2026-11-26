// Deferred settings window creation, synchronization and validation.
use super::*;

pub(super) struct Deferred<T>(pub(super) std::cell::OnceCell<T>);
impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self(std::cell::OnceCell::new())
    }
}
impl<T> Deferred<T> {
    pub(super) fn get(&self) -> Option<&T> {
        self.0.get()
    }
}
impl<T> std::ops::Deref for Deferred<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.get().expect("auxiliary window initialized before use")
    }
}

impl App {
    pub(super) fn show_settings(&mut self) {
        if !self.allowed(ShowSource::Settings) {
            return;
        }
        self.user_summon_edge();
        if self.settings_ui.get().is_none() {
            match create_settings(
                &self.weak_self.upgrade().expect("live application"),
                self.events.clone(),
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
        self.settings_ui.set_topmost_window(self.child_topmost());
        if let Err(error) = self.show_child_window(self.settings_ui.window()) {
            self.settings_open = false;
            self.status(error);
        } else {
            self.apply_theme();
        }
    }

    pub(super) fn close_settings(&mut self) {
        self.settings_ui.set_recording_hotkey(false);
        self.settings_ui.set_legal_open(false);
        self.settings_ui.set_status("".into());
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

    pub(super) fn sync_main_settings(&self) {
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

    pub(super) fn sync_settings_ui(&self) {
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

    pub(super) fn apply_settings(&mut self, field: i32) {
        if field == 3 {
            self.save_window(true);
            let mut state = self.saved_window.clone();
            state.always_on_top = self.settings_ui.get_keep_topmost();
            match self.store.save_window_state(&state) {
                Ok(()) => {
                    self.saved_window = state;
                    self.window_candidate = None;
                    self.apply_main_topmost(
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

    pub(super) fn add_protected_app(&mut self, path: String) {
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

    pub(super) fn remove_protected(&mut self, index: i32) {
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
}

const LICENSE_TEXT: &str = include_str!("../LICENSE");

fn notice_items() -> ModelRc<NoticeItem> {
    let items = [
        (
            "slint / i-slint-core / slint-build",
            "1.18.0",
            "GPL-3.0-only",
            "提供界面框架与 Fluent 软件渲染后端，并保留 AboutSlint 归属展示",
        ),
        (
            "rusqlite",
            "0.37.0",
            "MIT",
            "封装 SQLite 数据存储，并通过 bundled 特性内嵌公共领域版本",
        ),
        (
            "windows / windows-core",
            "0.62.2",
            "MIT OR Apache-2.0",
            "调用 Win32 窗口管理、Shell 交互、OLE 拖放、全局热键及网络请求接口",
        ),
        (
            "tray-icon",
            "0.24.2",
            "MIT OR Apache-2.0",
            "提供 Windows 系统托盘图标与原生托盘菜单",
        ),
        (
            "rfd",
            "0.16.0",
            "MIT",
            "调用系统原生的文件选择、目录选择与错误提示对话框",
        ),
        (
            "image",
            "0.25.10",
            "MIT OR Apache-2.0",
            "用于解析、缩放并编码 PNG、JPEG 与 ICO 格式图标",
        ),
        (
            "pinyin",
            "0.10.0",
            "MIT",
            "基于 plain 特性提供离线全拼与首字母匹配",
        ),
        (
            "arboard",
            "3.6.1",
            "MIT OR Apache-2.0",
            "用于将路径、网址、环境信息和许可证内容写入剪贴板",
        ),
        (
            "base64",
            "0.23.1",
            "MIT OR Apache-2.0",
            "负责 JSON 导出与恢复过程中的图标 Base64 编解码",
        ),
        (
            "serde / serde_json",
            "1.0.229 / 1.0.151",
            "MIT OR Apache-2.0",
            "负责设置项、本机窗口状态以及 JSON 导入导出的序列化",
        ),
        (
            "url",
            "2.5.8",
            "MIT OR Apache-2.0",
            "用于规范化网址并构造网页图标请求地址",
        ),
        (
            "thiserror",
            "2.0.20",
            "MIT OR Apache-2.0",
            "用于定义结构化的业务错误类型",
        ),
        (
            "raw-window-handle",
            "0.6.2",
            "MIT OR Apache-2.0 OR Zlib",
            "用于从窗口抽象中读取底层 Win32 HWND 句柄",
        ),
    ]
    .into_iter()
    .map(|(name, version, license, detail)| NoticeItem {
        name: name.into(),
        version: version.into(),
        license: license.into(),
        detail: detail.into(),
    })
    .collect::<Vec<_>>();
    Rc::new(VecModel::from(items)).into()
}

fn environment_summary() -> String {
    format!(
        "Raptim {} | Rust {} | Slint 1.18.0 (Fluent / Software) | SQLite WAL (v2) | Windows ({})",
        env!("CARGO_PKG_VERSION"),
        env!("RAPTIM_RUST_VERSION"),
        std::env::consts::ARCH
    )
}

fn create_settings(
    app: &Rc<RefCell<App>>,
    events: EventSink,
    data: &std::path::Path,
) -> std::result::Result<SettingsWindow, slint::PlatformError> {
    let settings_ui = SettingsWindow::new()?;
    settings_ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    settings_ui.set_rust_version(env!("RAPTIM_RUST_VERSION").into());
    settings_ui.set_license_lines(strings(LICENSE_TEXT.lines().map(str::to_string)));
    settings_ui.set_notice_items(notice_items());
    settings_ui.set_data_path(
        raptim::paths::normal_path(data)
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    settings_ui
        .window()
        .set_size(slint::LogicalSize::new(660., 480.));
    install_settings_keys(&settings_ui, events);
    macro_rules! bind {
        ($ui:ident, $callback:ident, |$s:ident $(,$arg:ident)*| $body:block) => {{
            let weak = Rc::downgrade(app);
            $ui.$callback(move |$($arg),*| {
                if let Some(app) = weak.upgrade() {
                    #[allow(unused_mut)]
                    let mut $s = app.borrow_mut();
                    $body
                }
            });
        }};
    }
    let bind_url = |url: &'static str| {
        move || {
            let _ = platform::launch(&Target::Url { url: url.into() });
        }
    };
    settings_ui.on_open_rust(bind_url("https://www.rust-lang.org/"));
    settings_ui.on_open_slint(bind_url("https://slint.dev/"));
    settings_ui.on_open_repo(bind_url("https://github.com/KHWLGH/Raptim"));
    settings_ui.on_open_issues(bind_url("https://github.com/KHWLGH/Raptim/issues"));
    settings_ui.on_open_changelog(bind_url(
        "https://github.com/KHWLGH/Raptim/blob/main/CHANGELOG.md",
    ));
    let copy_text = |ui: &SettingsWindow, text: String, ok_message: &'static str| {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => ui.set_status(ok_message.into()),
            Err(error) => ui.set_status(format!("复制失败：{error}").into()),
        }
    };
    let env_ui = settings_ui.as_weak();
    settings_ui.on_copy_env_info(move || {
        if let Some(ui) = env_ui.upgrade() {
            copy_text(&ui, environment_summary(), "已复制环境信息");
        }
    });
    let license_ui = settings_ui.as_weak();
    settings_ui.on_copy_license(move || {
        if let Some(ui) = license_ui.upgrade() {
            copy_text(&ui, LICENSE_TEXT.to_string(), "已复制 GPL-3.0 许可证全文");
        }
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
        s.dialog(DialogKind::ProtectedApp);
    });
    bind!(settings_ui, on_add_protected_app, |s, value| {
        s.add_protected_app(value.to_string());
        s.settings_ui.set_protected_input("".into());
    });
    bind!(settings_ui, on_remove_protected_app, |s, index| {
        s.remove_protected(index);
    });
    bind!(settings_ui, on_export_data, |s| {
        s.dialog(DialogKind::ExportJson);
    });
    bind!(settings_ui, on_import_data, |s| {
        s.dialog(DialogKind::RestoreJson);
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
