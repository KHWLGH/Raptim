use crate::{
    launch::LaunchError,
    model::{Settings, ShowMode, Target},
    policy::{Notification, Protection, Signals, decide},
};
use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, mpsc},
    thread,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::*, Gdi::*},
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        System::{Com::*, ProcessStatus::*, Threading::*},
        UI::{
            Accessibility::*, Controls::MARGINS, Input::KeyboardAndMouse::*, Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
/// An item id list the Shell allocated from the task memory allocator.
struct TaskMemIdl(*mut Common::ITEMIDLIST);
impl Drop for TaskMemIdl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 as *const _)) }
    }
}

pub struct Instance {
    _mutex: OwnedHandle,
    event: OwnedHandle,
    stop: OwnedHandle,
    waiter: Option<thread::JoinHandle<()>>,
}
impl Instance {
    /// One instance per data directory; no existing instance can be bypassed by a second writer.
    pub fn acquire(data_dir: &Path) -> Result<Option<Self>, String> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
        let prefix = format!("Local\\Diascia-{:016x}", hasher.finish());
        let mutex_name = wide(&prefix);
        let event_name = wide(&format!("{prefix}-show"));
        unsafe {
            let mutex = OwnedHandle(
                CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))
                    .map_err(|e| e.to_string())?,
            );
            let exists = GetLastError() == ERROR_ALREADY_EXISTS;
            let event = OwnedHandle(
                CreateEventW(None, false, false, PCWSTR(event_name.as_ptr()))
                    .map_err(|e| e.to_string())?,
            );
            if exists {
                SetEvent(event.0).map_err(|e| e.to_string())?;
                return Ok(None);
            }
            Ok(Some(Self {
                _mutex: mutex,
                event,
                stop: OwnedHandle(
                    CreateEventW(None, true, false, None).map_err(|e| e.to_string())?,
                ),
                waiter: None,
            }))
        }
    }
    pub fn listen(&mut self, notify: Notify) {
        if self.waiter.is_some() {
            return;
        }
        let event = self.event.0.0 as usize;
        let stop = self.stop.0.0 as usize;
        self.waiter = Some(thread::spawn(move || unsafe {
            let handles = [HANDLE(stop as _), HANDLE(event as _)];
            loop {
                let result = WaitForMultipleObjects(&handles, false, INFINITE);
                if result != WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
                    break;
                }
                crate::metrics::record("secondary_received", 0.);
                notify(7, String::new());
            }
        }));
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop.0);
        }
        if let Some(worker) = self.waiter.take() {
            let _ = worker.join();
        }
    }
}

/// Caches the foreground process image path so repeated polling does not
/// reopen the process handle while the window stays unchanged.
#[derive(Default)]
pub struct ProtectionCache {
    hwnd: usize,
    pid: u32,
    path: String,
}

/// AppsFolder gives one display name plus one parsing name per child; only entries that parse to
/// an app id are usable as a launch target, so everything else (virtual sub-folders in
/// particular) is dropped here rather than becoming a broken project.
fn app_option(display: &str, parsing: &str) -> Option<(String, String)> {
    // The parent folder hands back a child-relative parsing name, which is the bare app id; the
    // absolute form is accepted too because that is what a dropped Shell item reports.
    let aumid = parsing
        .strip_prefix("shell:AppsFolder\\")
        .unwrap_or(parsing)
        .trim_end_matches('\\');
    if !aumid.contains('!') || aumid.contains('\\') || display.trim().is_empty() {
        return None;
    }
    Some((aumid.to_string(), display.to_string()))
}

/// Installed packaged applications as `(aumid, display name)`.
pub fn installed_apps() -> Vec<(String, String)> {
    // Best effort: a thread that already initialised a different apartment model still resolves
    // shell items, and a thread with no COM at all simply gets an empty list back.
    let _com = ComGuard::new().ok();
    unsafe {
        let Ok(item): windows::core::Result<IShellItem> =
            SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)
        else {
            return Vec::new();
        };
        let Ok(folder): windows::core::Result<IShellFolder> =
            item.BindToHandler(None, &BHID_SFObject)
        else {
            return Vec::new();
        };
        let mut enumerator: Option<IEnumIDList> = None;
        if folder
            .EnumObjects(
                HWND::default(),
                (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INIT_ON_FIRST_NEXT.0) as u32,
                &mut enumerator,
            )
            .is_err()
        {
            return Vec::new();
        }
        let Some(enumerator) = enumerator else {
            return Vec::new();
        };
        let mut apps = Vec::new();
        let mut buffer = [std::ptr::null_mut::<Common::ITEMIDLIST>(); 1];
        while enumerator.Next(&mut buffer, None) == S_OK {
            let child = buffer[0];
            if child.is_null() {
                continue;
            }
            let display = child_name(&folder, child, SHGDN_NORMAL, 512);
            let parsing = child_name(&folder, child, SHGDN_FORPARSING, 4096);
            CoTaskMemFree(Some(child as *const _));
            if let Some(option) = app_option(&display, &parsing) {
                apps.push(option);
            }
        }
        apps.sort_by_key(|app| app.1.to_lowercase());
        apps
    }
}

fn child_name(
    folder: &IShellFolder,
    child: *const Common::ITEMIDLIST,
    flag: SHGDNF,
    width: usize,
) -> String {
    let mut wide = vec![0u16; width];
    let mut str_ret = Common::STRRET::default();
    if unsafe { folder.GetDisplayNameOf(child, flag, &mut str_ret) }.is_err()
        || unsafe { StrRetToBufW(&mut str_ret, Some(child), &mut wide) }.is_err()
    {
        return String::new();
    }
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

pub fn protection(settings: &Settings) -> Protection {
    let mut cache = ProtectionCache::default();
    decide(collect_signals(settings, &mut cache))
}

/// The raw observations behind one verdict. The existing deny-list, per-application rule and
/// `rcMonitor` geometry work stays in `protection_cached`; the system notification state is
/// added as the third signal and is only queried while fullscreen protection is switched on, so
/// it can never resurrect a protection the user turned off.
pub fn collect_signals(settings: &Settings, cache: &mut ProtectionCache) -> Signals {
    let observed = protection_cached(settings, cache);
    Signals {
        strict_rule: observed == Protection::Strict,
        fullscreen_geometry: observed == Protection::Fullscreen,
        notification: if settings.fullscreen_protection {
            notification_state()
        } else {
            Notification::Unavailable
        },
        ..Default::default()
    }
}

pub fn notification_state() -> Notification {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(QUNS_RUNNING_D3D_FULL_SCREEN) => Notification::RunningD3dFullScreen,
        Ok(QUNS_PRESENTATION_MODE) => Notification::PresentationMode,
        Ok(QUNS_BUSY) => Notification::Busy,
        Ok(_) => Notification::Other,
        Err(_) => Notification::Unavailable,
    }
}

pub fn protection_cached(settings: &Settings, cache: &mut ProtectionCache) -> Protection {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null()
            || hwnd == GetDesktopWindow()
            || hwnd == GetShellWindow()
            || !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
        {
            return Protection::Normal;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class);
        if matches!(
            String::from_utf16_lossy(&class[..len.max(0) as usize]).as_str(),
            "WorkerW"
                | "Progman"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "XamlExplorerHostIslandWindow"
                | "MultitaskingViewFrame"
                | "TaskSwitcherWnd"
        ) {
            return Protection::Normal;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return Protection::Normal;
        }
        if !settings.protected_apps.is_empty() {
            // Both keys must match: a recycled pid under a different window must re-query.
            let cached = cache.hwnd == hwnd.0 as usize && cache.pid == pid;
            let path = if cached {
                Some(cache.path.clone())
            } else {
                let queried = query_image_path(pid);
                if let Some(path) = &queried {
                    *cache = ProtectionCache {
                        hwnd: hwnd.0 as usize,
                        pid,
                        path: path.clone(),
                    };
                }
                queried
            };
            if path.is_some_and(|path| {
                settings
                    .protected_apps
                    .iter()
                    .any(|rule| rule.replace('/', "\\").to_lowercase() == path)
            }) {
                return Protection::Strict;
            }
        }
        if !settings.fullscreen_protection {
            return Protection::Normal;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if IsZoomed(hwnd).as_bool()
            || style & WS_CAPTION.0 == WS_CAPTION.0
            || ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0
        {
            return Protection::Normal;
        }
        let mut rect = RECT::default();
        // DWM frame bounds are physical pixels with at most a small border offset
        // (magnified by display scaling), while GetWindowRect includes the
        // invisible resize frame of thick-framed windows.
        let mut tolerance = 8;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
        {
            let mut measured = false;
            let mut client = RECT::default();
            if GetClientRect(hwnd, &mut client).is_ok() {
                let mut top_left = POINT {
                    x: client.left,
                    y: client.top,
                };
                let mut bottom_right = POINT {
                    x: client.right,
                    y: client.bottom,
                };
                if ClientToScreen(hwnd, &mut top_left).as_bool()
                    && ClientToScreen(hwnd, &mut bottom_right).as_bool()
                {
                    rect = RECT {
                        left: top_left.x,
                        top: top_left.y,
                        right: bottom_right.x,
                        bottom: bottom_right.y,
                    };
                    measured = true;
                }
            }
            if !measured {
                if GetWindowRect(hwnd, &mut rect).is_err() {
                    return Protection::Normal;
                }
                tolerance = 24;
            }
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool()
            && fullscreen_rect(rect, info.rcMonitor, tolerance)
        {
            return Protection::Fullscreen;
        }
        Protection::Normal
    }
}

fn query_image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let handle = OwnedHandle(handle);
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .ok()?;
        Some(
            String::from_utf16_lossy(&buffer[..length as usize])
                .replace('/', "\\")
                .to_lowercase(),
        )
    }
}

fn fullscreen_rect(rect: RECT, screen: RECT, tolerance: i32) -> bool {
    (rect.left - screen.left).abs() <= tolerance
        && (rect.top - screen.top).abs() <= tolerance
        && (rect.right - screen.right).abs() <= tolerance
        && (rect.bottom - screen.bottom).abs() <= tolerance
}

struct ComGuard;
impl ComGuard {
    fn new() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        Ok(Self)
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

/// The window state the Shell should start the target in. Lives here so `model.rs`
/// never has to name a Win32 type.
pub fn show_cmd(mode: ShowMode) -> SHOW_WINDOW_CMD {
    match mode {
        ShowMode::Normal => SW_SHOWNORMAL,
        ShowMode::Maximized => SW_SHOWMAXIMIZED,
        ShowMode::Minimized => SW_SHOWMINIMIZED,
    }
}

/// Run on a worker thread: Shell extensions and unavailable network paths must not block UI.
pub fn launch(target: &Target) -> Result<(), LaunchError> {
    target
        .validate()
        .map_err(|e| LaunchError::Invalid(e.to_string()))?;
    let _com = ComGuard::new().map_err(|e| LaunchError::Other(format!("COM 初始化失败：{e}")))?;
    unsafe {
        if let Target::App { aumid } = target {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| LaunchError::Other(format!("无法创建应用激活器：{e}")))?;
            let app = wide(aumid);
            // ActivateApplication reports catalog problems as facility-bearing HRESULTs whose
            // low word collides with unrelated Win32 codes, so classify it as its own case.
            manager
                .ActivateApplication(PCWSTR(app.as_ptr()), w!(""), AO_NONE)
                .map_err(|_| LaunchError::AppNotInstalled)?;
            return Ok(());
        }
        let target_string = wide(target.value());
        let (arguments, directory, elevated, shown) = match target {
            Target::Program {
                arguments,
                working_dir,
                elevated,
                show,
                ..
            } => (
                wide(arguments),
                wide(working_dir),
                *elevated,
                show_cmd(*show),
            ),
            Target::Shortcut { show, .. } => (wide(""), wide(""), false, show_cmd(*show)),
            _ => (wide(""), wide(""), false, show_cmd(ShowMode::Normal)),
        };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: if elevated { w!("runas") } else { w!("open") },
            lpFile: PCWSTR(target_string.as_ptr()),
            lpParameters: if arguments.len() > 1 {
                PCWSTR(arguments.as_ptr())
            } else {
                PCWSTR::null()
            },
            lpDirectory: if directory.len() > 1 {
                PCWSTR(directory.as_ptr())
            } else {
                PCWSTR::null()
            },
            nShow: shown.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info)
            .map_err(|e| LaunchError::from_os_error(e.code().0 as u32 & 0xffff))
    }
}

pub type Notify = Arc<dyn Fn(i32, String) + Send + Sync>;
#[path = "frame_clock.rs"]
mod frame_clock;
pub use frame_clock::FrameClock;
const HOTKEY_CONTROL: u32 = WM_APP + 11;
struct HotkeyRequest {
    enabled: bool,
    spec: crate::hotkey::Shortcut,
    reply: mpsc::SyncSender<Result<(), String>>,
}
pub struct Hotkey {
    thread_id: u32,
    join: Option<thread::JoinHandle<()>>,
    enabled: bool,
    spec: crate::hotkey::Shortcut,
    sender: mpsc::Sender<HotkeyRequest>,
}
impl Hotkey {
    pub fn new(notify: Notify, enabled: bool, shortcut: &str) -> Result<Self, String> {
        let spec = crate::hotkey::parse(shortcut)?;
        let (tx, rx) = mpsc::sync_channel(1);
        let (sender, receiver) = mpsc::channel::<HotkeyRequest>();
        let worker_notify = notify.clone();
        let join = thread::spawn(move || unsafe {
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            let _ = tx.send(GetCurrentThreadId());
            let mut registered: Option<(i32, crate::hotkey::Shortcut)> = None;
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                if message.message == HOTKEY_CONTROL {
                    while let Ok(request) = receiver.try_recv() {
                        let same = registered
                            .as_ref()
                            .is_some_and(|(_, old)| *old == request.spec);
                        let result = if same {
                            if !request.enabled {
                                let _ = UnregisterHotKey(None, registered.take().unwrap().0);
                            }
                            Ok(())
                        } else {
                            let id = if registered.as_ref().is_some_and(|(id, _)| *id == 1) {
                                2
                            } else {
                                1
                            };
                            // Register the replacement first, retaining the old binding on conflict.
                            match RegisterHotKey(
                                None,
                                id,
                                HOT_KEY_MODIFIERS(request.spec.modifiers) | MOD_NOREPEAT,
                                request.spec.key,
                            ) {
                                Ok(()) => {
                                    if let Some((old, _)) = registered.take() {
                                        let _ = UnregisterHotKey(None, old);
                                    }
                                    if request.enabled {
                                        registered = Some((id, request.spec));
                                    } else {
                                        let _ = UnregisterHotKey(None, id);
                                    }
                                    Ok(())
                                }
                                Err(_) => Err("此呼出热键已被占用或不可用，已保留原热键".into()),
                            }
                        };
                        let _ = request.reply.send(result);
                    }
                } else if message.message == WM_HOTKEY
                    && registered
                        .as_ref()
                        .is_some_and(|(id, _)| *id as usize == message.wParam.0)
                {
                    crate::metrics::record("hotkey_received", 0.);
                    worker_notify(1, String::new());
                }
            }
            if let Some((id, _)) = registered {
                let _ = UnregisterHotKey(None, id);
            }
        });
        let mut hotkey = Self {
            thread_id: rx.recv().map_err(|e| e.to_string())?,
            join: Some(join),
            enabled: false,
            spec,
            sender,
        };
        if let Err(error) = hotkey.configure(shortcut, enabled) {
            notify(2, format!("{error}；可用托盘找回窗口"));
        }
        Ok(hotkey)
    }
    pub fn configure(&mut self, shortcut: &str, enabled: bool) -> Result<(), String> {
        let spec = crate::hotkey::parse(shortcut)?;
        let (reply, rx) = mpsc::sync_channel(1);
        self.sender
            .send(HotkeyRequest {
                enabled,
                spec: spec.clone(),
                reply,
            })
            .map_err(|e| e.to_string())?;
        unsafe {
            PostThreadMessageW(self.thread_id, HOTKEY_CONTROL, WPARAM(0), LPARAM(0))
                .map_err(|e| e.to_string())?;
        }
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| e.to_string())??;
        self.spec = spec;
        self.enabled = enabled;
        Ok(())
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            let label = self.spec.label.clone();
            let _ = self.configure(&label, enabled);
        }
    }
}
impl Drop for Hotkey {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

thread_local! {
    static FOREGROUND_NOTIFY: std::cell::RefCell<Option<Notify>> =
        const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_thread: u32,
    _time: u32,
) {
    let interesting = matches!(
        event,
        EVENT_SYSTEM_FOREGROUND | EVENT_SYSTEM_MINIMIZEEND | EVENT_SYSTEM_MOVESIZEEND
    );
    // Anything that is not the window itself (a caret, a child item) fires far too often to be
    // a protection signal.
    if interesting && id_object == OBJID_WINDOW.0 && id_child == 0 {
        FOREGROUND_NOTIFY.with(|slot| {
            if let Some(notify) = slot.borrow().as_ref() {
                crate::metrics::record("foreground_received", 0.);
                notify(14, String::new());
            }
        });
    }
}

/// Resolution and mode changes reach `HWND_BROADCAST`, which a message-only window never
/// receives, so this watches from an ordinary hidden top-level window on the pump thread.
unsafe extern "system" fn display_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DISPLAYCHANGE {
        FOREGROUND_NOTIFY.with(|slot| {
            if let Some(notify) = slot.borrow().as_ref() {
                crate::metrics::record("display_changed", 0.);
                notify(17, String::new());
            }
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

const DISPLAY_CLASS: PCWSTR = w!("Diascia.DisplayWatch");

/// Delivers an immediate re-evaluation whenever the foreground window changes,
/// instead of waiting for the polling timer.
pub struct ForegroundWatch {
    thread_id: u32,
    join: Option<thread::JoinHandle<()>>,
}
impl ForegroundWatch {
    pub fn new(notify: Notify) -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        let join = thread::spawn(move || unsafe {
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            FOREGROUND_NOTIFY.with(|slot| *slot.borrow_mut() = Some(notify));
            // Out-of-context hooks require the installing thread to pump messages,
            // which the loop below provides.
            let hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(foreground_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            let instance = HINSTANCE::default();
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(display_proc),
                hInstance: instance,
                lpszClassName: DISPLAY_CLASS,
                ..Default::default()
            };
            let registered = RegisterClassExW(&class);
            let watch = if registered != 0 {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    DISPLAY_CLASS,
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    None,
                )
                .unwrap_or_default()
            } else {
                HWND::default()
            };
            let _ = tx.send(GetCurrentThreadId());
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {}
            if !watch.0.is_null() {
                let _ = DestroyWindow(watch);
            }
            if registered != 0 {
                let _ = UnregisterClassW(DISPLAY_CLASS, instance.into());
            }
            if !hook.0.is_null() {
                let _ = UnhookWinEvent(hook);
            }
            FOREGROUND_NOTIFY.with(|slot| *slot.borrow_mut() = None);
        });
        let thread_id = rx.recv().map_err(|e| e.to_string())?;
        Ok(Self {
            thread_id,
            join: Some(join),
        })
    }
}
impl Drop for ForegroundWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub struct Tray {
    _icon: tray_icon::TrayIcon,
    pause_hide: tray_icon::menu::CheckMenuItem,
    pause_protection: tray_icon::menu::CheckMenuItem,
}
impl Tray {
    pub fn new(notify: Notify) -> Result<Self, String> {
        use tray_icon::{
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
            menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
        };
        let menu = Menu::new();
        let show = MenuItem::new("显示 Diascia", true, None);
        let restore = MenuItem::new("恢复窗口位置", true, None);
        let pause_hide = CheckMenuItem::new("暂停自动隐藏", true, false, None);
        let pause_protection = CheckMenuItem::new("暂停游戏勿扰", true, false, None);
        let settings = MenuItem::new("设置", true, None);
        let exit = MenuItem::new("退出", true, None);
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
            let code = if event.id == ids.0 {
                3
            } else if event.id == ids.1 {
                6
            } else if event.id == ids.2 {
                4
            } else if event.id == ids.3 {
                5
            } else if event.id == ids.4 {
                15
            } else if event.id == ids.5 {
                16
            } else {
                return;
            };
            menu_notify(code, String::new());
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
                notify(3, String::new());
            }
        }));
        // Original small flower mark; no concept-image assets are shipped.
        let mut rgba = vec![0u8; 32 * 32 * 4];
        for y in 0..32i32 {
            for x in 0..32i32 {
                let petal = [(11, 10), (21, 10), (8, 19), (24, 19), (16, 25)]
                    .iter()
                    .any(|(cx, cy)| (x - cx).pow(2) + (y - cy).pow(2) < 30);
                let center = (x - 16).pow(2) + (y - 16).pow(2) < 15;
                if petal || center {
                    let p = ((y * 32 + x) * 4) as usize;
                    rgba[p..p + 4].copy_from_slice(if center {
                        &[255, 222, 120, 255]
                    } else {
                        &[172, 108, 210, 255]
                    });
                }
            }
        }
        let icon = Icon::from_rgba(rgba, 32, 32).map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("Diascia")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _icon: icon,
            pause_hide,
            pause_protection,
        })
    }
    /// Keeps the two check marks honest after a toggle from any other entry point.
    pub fn set_states(&self, paused: bool, protection_bypassed: bool) {
        self.pause_hide.set_checked(paused);
        self.pause_protection.set_checked(protection_bypassed);
    }
}

pub fn set_owner(child: &slint::Window, parent: &slint::Window) {
    let (Some(owner), Some(child_hwnd)) = (native_handle(parent), native_handle(child)) else {
        return;
    };
    unsafe {
        SetWindowLongPtrW(child_hwnd, GWLP_HWNDPARENT, owner.0 as isize);
        let pinned = GetWindowLongPtrW(owner, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
        let _ = SetWindowPos(
            child_hwnd,
            Some(if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn focus(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
}

/// Position in physical pixels on the parent's monitor, including negative origins.
pub fn center_child(child: &slint::Window, parent: &slint::Window) {
    use crate::windowing::{Rect, centered_position};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    let geometry = parent
        .with_winit_window(|w| {
            let RawWindowHandle::Win32(h) = w.window_handle().ok()?.as_raw() else {
                return None;
            };
            unsafe {
                let hwnd = HWND(h.hwnd.get() as _);
                let mut rect = RECT::default();
                GetWindowRect(hwnd, &mut rect).ok()?;
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                    .as_bool()
                {
                    return None;
                }
                let convert = |r: RECT| Rect {
                    x: r.left,
                    y: r.top,
                    width: r.right - r.left,
                    height: r.bottom - r.top,
                };
                Some((convert(rect), convert(info.rcWork)))
            }
        })
        .flatten();
    if let Some((parent_rect, work)) = geometry {
        let size = outer_size_of(child)
            .map(|(width, height)| slint::PhysicalSize::new(width, height))
            .unwrap_or_else(|| {
                child
                    .size()
                    .to_logical(child.scale_factor())
                    .to_physical(parent.scale_factor())
            });
        let (x, y) = centered_position(parent_rect, (size.width as i32, size.height as i32), work);
        child.set_position(slint::PhysicalPosition::new(x, y));
    }
}
pub fn restore_position(window: &slint::Window) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    let Some((width, height)) = rect_size(hwnd) else {
        return;
    };
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool()
        {
            return;
        }
        let rect = info.rcWork;
        let x = rect.left + ((rect.right - rect.left - width as i32) / 2).max(0);
        let y = rect.top + ((rect.bottom - rect.top - height as i32) / 2).max(0);
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn cursor() -> (i32, i32) {
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        (p.x, p.y)
    }
}
pub fn mouse_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}
pub fn key_modifiers() -> u32 {
    unsafe {
        let mut flags = 0;
        for (key, bit) in [
            (VK_CONTROL, 2),
            (VK_MENU, 1),
            (VK_SHIFT, 4),
            (VK_LWIN, 8),
            (VK_RWIN, 8),
        ] {
            if GetKeyState(key.0 as i32) < 0 {
                flags |= bit;
            }
        }
        flags
    }
}
fn native_handle(window: &slint::Window) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|w| {
            if let RawWindowHandle::Win32(h) = w.window_handle().ok()?.as_raw() {
                Some(HWND(h.hwnd.get() as _))
            } else {
                None
            }
        })
        .flatten()
}
pub fn native_ready(window: &slint::Window) -> bool {
    native_handle(window).is_some_and(|hwnd| unsafe { IsWindow(Some(hwnd)).as_bool() })
}

/// Outer size in physical pixels, or None once the native handle is gone. `winit::Window::
/// outer_size` panics in that case, and a dead handle is reachable here: Windows destroys an
/// owned window together with its owner, while Slint keeps the component alive.
fn rect_size(hwnd: HWND) -> Option<(u32, u32)> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok().map(|_| {
            (
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            )
        })
    }
}
fn outer_size_of(window: &slint::Window) -> Option<(u32, u32)> {
    native_handle(window).and_then(rect_size)
}

/// Restore an existing hidden edge window without activating it or revealing
/// its expanded rectangle. Geometry and clipping must be prepared beforehand.
pub fn show_without_activation(window: &slint::Window) -> Result<(), slint::PlatformError> {
    let Some(hwnd) = native_handle(window) else {
        return Ok(());
    };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
        let result = window.show();
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        result
    }
}
// Suppress the legacy non-client renderer while retaining the sizing style
// DWM requires for a composited outline. Never add WS_CAPTION to a Slint window.
struct LauncherDrag {
    cursor: POINT,
    origin: POINT,
}
unsafe fn clear_launcher_drag(hwnd: HWND) {
    unsafe {
        let pointer =
            RemovePropW(hwnd, w!("Diascia.Drag")).unwrap_or_default().0 as *mut LauncherDrag;
        if !pointer.is_null() {
            drop(Box::from_raw(pointer));
        }
    }
}
/// Capture pointer movement without entering Windows' SC_MOVE / Aero Snap loop.
pub fn begin_launcher_drag(window: &slint::Window) {
    // show() before the event loop can precede creation of the HWND. Never
    // assume the startup decoration call installed the launcher message path.
    ensure_launcher(window);
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        clear_launcher_drag(hwnd);
        let mut cursor = POINT::default();
        let mut rect = RECT::default();
        if GetCursorPos(&mut cursor).is_err() || GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let pointer = Box::into_raw(Box::new(LauncherDrag {
            cursor,
            origin: POINT {
                x: rect.left,
                y: rect.top,
            },
        }));
        if SetPropW(hwnd, w!("Diascia.Drag"), Some(HANDLE(pointer as _))).is_err() {
            drop(Box::from_raw(pointer));
            return;
        }
        // Winit usually owns capture already after the left-button press.
        // Reacquiring it can send WM_CAPTURECHANGED with this same HWND.
        if GetCapture() != hwnd {
            SetCapture(hwnd);
        }
    }
}
unsafe extern "system" fn frame_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        let launcher = !GetPropW(hwnd, w!("Diascia.Launcher")).is_invalid();
        if launcher {
            match message {
                WM_SYSCOMMAND if matches!(wparam.0 as u32 & 0xfff0, SC_MAXIMIZE | SC_MOVE) => {
                    return LRESULT(0);
                }
                WM_NCLBUTTONDBLCLK if wparam.0 as u32 == HTCAPTION => return LRESULT(0),
                WM_MOUSEMOVE => {
                    let pointer = GetPropW(hwnd, w!("Diascia.Drag")).0 as *const LauncherDrag;
                    if !pointer.is_null() {
                        let drag = &*pointer;
                        let mut cursor = POINT::default();
                        if GetCursorPos(&mut cursor).is_ok() {
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                drag.origin.x + cursor.x - drag.cursor.x,
                                drag.origin.y + cursor.y - drag.cursor.y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                        return LRESULT(0);
                    }
                }
                WM_LBUTTONUP | WM_CANCELMODE => {
                    if !GetPropW(hwnd, w!("Diascia.Drag")).is_invalid() {
                        clear_launcher_drag(hwnd);
                        let _ = ReleaseCapture();
                        // Winit/Slint must also see the release to reset their
                        // press/capture counters for the next drag or click.
                    }
                }
                WM_CAPTURECHANGED if lparam.0 != hwnd.0 as isize => clear_launcher_drag(hwnd),
                _ => {}
            }
        }
        match message {
            WM_NCCALCSIZE | WM_NCPAINT => LRESULT(0),
            WM_NCACTIVATE => LRESULT(1),
            WM_NCDESTROY => {
                clear_launcher_drag(hwnd);
                let _ = RemoveWindowSubclass(hwnd, Some(frame_proc), id);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }
}
pub fn decorate(window: &slint::Window) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        let _ = RemovePropW(hwnd, w!("Diascia.EdgeClip"));
        enable_custom_frame(hwnd);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (ex & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize,
        );
        use slint::winit_030::{
            WinitWindowAccessor,
            winit::window::{Theme, WindowButtons},
        };
        window.with_winit_window(|w| {
            // 窗口隐藏到托盘/贴边即为关闭语义；最小化会让跳过任务栏的窗口无法找回。
            w.set_enabled_buttons(WindowButtons::CLOSE);
        });
        let dark = windows::core::BOOL::from(
            window
                .with_winit_window(|w| w.theme() == Some(Theme::Dark))
                .unwrap_or(false),
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            4,
        );
        let corner = DWMWCP_ROUND;
        let rounded = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as _,
            4,
        )
        .is_ok();
        let policy = DWMNCRP_ENABLED;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, &policy as *const _ as _, 4);
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
        refresh_border(window);
        if rounded {
            // Let the compositor anti-alias corners. An HRGN here disables DWM
            // rounding and creates visibly jagged edges, especially at high DPI.
            let _ = SetWindowRgn(hwnd, None, false);
        } else {
            let size = window.size();
            let diameter = (16. * window.scale_factor()).round() as i32;
            let region = CreateRoundRectRgn(
                0,
                0,
                size.width as i32,
                size.height as i32,
                diameter,
                diameter,
            );
            if SetWindowRgn(hwnd, Some(region), false) == 0 {
                let _ = DeleteObject(region.into());
            }
        }
    }
}
pub fn refresh_geometry(window: &slint::Window) {
    if native_handle(window)
        .is_some_and(|hwnd| unsafe { GetPropW(hwnd, w!("Diascia.EdgeClip")).is_invalid() })
    {
        decorate(window);
    }
}
unsafe fn enable_custom_frame(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(frame_proc), 0xD1A5, 0);
        let old = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let mut style = (old & !(WS_CAPTION.0 as isize)) | WS_THICKFRAME.0 as isize;
        if !GetPropW(hwnd, w!("Diascia.Launcher")).is_invalid() {
            style &= !(WS_MAXIMIZEBOX.0 as isize);
        }
        if old != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, style);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}
pub fn refresh_border(window: &slint::Window) {
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            // DWMWA_COLOR_NONE: no accent tint or active/inactive color changes.
            let color = COLORREF(0xfffffffe);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &color as *const _ as _, 4);
        }
    }
}
pub fn work_area_at(point: (i32, i32)) -> Option<crate::windowing::Rect> {
    unsafe {
        let monitor = MonitorFromPoint(
            POINT {
                x: point.0,
                y: point.1,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcWork;
        Some(crate::windowing::Rect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        })
    }
}
pub fn move_clipped(window: &slint::Window, position: (i32, i32), work: crate::windowing::Rect) {
    use slint::winit_030::WinitWindowAccessor;
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    let size = window.size();
    let destination = crate::windowing::Rect {
        x: position.0,
        y: position.1,
        width: size.width as i32,
        height: size.height as i32,
    };
    // Once a docked slide needs a work-area region, keep that same outline when
    // fully expanded. Swapping an aliased region for DWM on the last frame makes
    // corners and shadows pop. Undocking explicitly calls decorate().
    let retained_clip = unsafe { GetPropW(hwnd, w!("Diascia.EdgeClip")).0 as usize == 2 };
    let clip = retained_clip
        || window
            .with_winit_window(|w| {
                let monitors: Vec<_> = w
                    .available_monitors()
                    .map(|monitor| {
                        let p = monitor.position();
                        let s = monitor.size();
                        crate::windowing::Rect {
                            x: p.x,
                            y: p.y,
                            width: s.width as i32,
                            height: s.height as i32,
                        }
                    })
                    .collect();
                monitors.is_empty() || crate::docking::needs_work_clip(destination, work, monitors)
            })
            .unwrap_or(true);
    unsafe {
        // 1: off-desktop slide with compositor corners; 2: explicit work-area HRGN.
        let previous = GetPropW(hwnd, w!("Diascia.EdgeClip")).0 as usize;
        let mut old = RECT::default();
        if GetWindowRect(hwnd, &mut old).is_ok()
            && (old.left, old.top) == position
            && previous == 1
            && !clip
        {
            return;
        }
        if !clip {
            let _ = SetPropW(hwnd, w!("Diascia.EdgeClip"), Some(HANDLE(1usize as _)));
        }
        // Suppress intermediate paints until the destination region has been applied.
        let _ = SetWindowPos(
            hwnd,
            None,
            position.0,
            position.1,
            0,
            0,
            SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOACTIVATE
                | if clip {
                    SWP_NOREDRAW
                } else {
                    SET_WINDOW_POS_FLAGS(0)
                },
        );
    }
    if clip {
        clip_to_work_area(window, work);
    }
}
/// Complete a docked slide without rebuilding its frame/corners on the last tick.
pub fn finish_edge_motion(window: &slint::Window) {
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            if GetPropW(hwnd, w!("Diascia.EdgeClip")).0 as usize == 1 {
                let _ = RemovePropW(hwnd, w!("Diascia.EdgeClip"));
            }
        }
    }
}
/// (working set, private bytes) in KiB, for the opt-in metrics CSV. Recorded beside
/// `entry_blob_bytes` so a memory reading always carries the dataset it was taken on.
pub fn memory_kib() -> (u64, u64) {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            std::ptr::addr_of_mut!(counters).cast(),
            counters.cb,
        )
    };
    if ok.as_bool() {
        (
            counters.WorkingSetSize as u64 / 1024,
            counters.PrivateUsage as u64 / 1024,
        )
    } else {
        (0, 0)
    }
}
/// Native clip prevents a sliding window leaking onto an adjacent display.
pub fn clip_to_work_area(window: &slint::Window, work: crate::windowing::Rect) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    crate::metrics::record("region_set", 1.);
    unsafe {
        let first = GetPropW(hwnd, w!("Diascia.EdgeClip")).0 as usize != 2;
        let _ = SetPropW(hwnd, w!("Diascia.EdgeClip"), Some(HANDLE(2usize as _)));
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        if first {
            let corner = DWMWCP_DONOTROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as _,
                4,
            );
            let border = COLORREF(0xfffffffe);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &border as *const _ as _, 4);
            let policy = DWMNCRP_DISABLED;
            let _ =
                DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, &policy as *const _ as _, 4);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
        }
        let left = r.left.max(work.x);
        let top = r.top.max(work.y);
        let right = r.right.min(work.x + work.width).max(left);
        let bottom = r.bottom.min(work.y + work.height).max(top);
        let region = CreateRectRgn(left - r.left, top - r.top, right - r.left, bottom - r.top);
        if right > left && bottom > top {
            // A slide translates a fixed shape. Scaling its radius by the visible
            // fraction makes corners inflate and snap when DWM takes over again.
            let diameter = (16. * window.scale_factor()).round() as i32;
            let rounded = CreateRoundRectRgn(
                0,
                0,
                r.right - r.left + 1,
                r.bottom - r.top + 1,
                diameter,
                diameter,
            );
            let _ = CombineRgn(Some(region), Some(region), Some(rounded), RGN_AND);
            let _ = DeleteObject(rounded.into());
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
        }
    }
}
pub fn set_opacity(window: &slint::Window, opacity: f32) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        let alpha = (opacity.clamp(0., 1.) * 255.).round() as usize;
        if GetPropW(hwnd, w!("Diascia.Alpha")).0 as usize == alpha + 1 {
            return;
        }
        let _ = SetPropW(hwnd, w!("Diascia.Alpha"), Some(HANDLE((alpha + 1) as _)));
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if opacity >= 1.0 {
            if style & WS_EX_LAYERED.0 as isize != 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED.0 as isize));
                window.request_redraw();
            }
        } else {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
            let _ = SetLayeredWindowAttributes(
                hwnd,
                COLORREF(0),
                (opacity.clamp(0., 1.) * 255.) as u8,
                LWA_ALPHA,
            );
        }
    }
}
pub fn animations_enabled() -> bool {
    let mut enabled = windows::core::BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
}
pub fn reveal_file(path: &Path) -> Result<(), String> {
    let _com = ComGuard::new()?;
    let path = crate::paths::normal_path(path);
    if !path.exists() {
        return Err(format!("目标已不存在：{}", path.display()));
    }
    unsafe {
        let name = wide(&path.to_string_lossy());
        let pidl = ILCreateFromPathW(PCWSTR(name.as_ptr()));
        if pidl.is_null() {
            return Err("无法解析目标所在位置".into());
        }
        // A fully qualified PIDL handles Unicode, commas, UNC and directories,
        // without Explorer command-line parsing or incorrectly opening the directory itself.
        let result =
            SHOpenFolderAndSelectItems(pidl, None, 0).map_err(|e| format!("无法打开所在位置：{e}"));
        ILFree(Some(pidl));
        result
    }
}
pub fn topmost(window: &slint::Window, enabled: bool) {
    use slint::winit_030::{WinitWindowAccessor, winit::window::WindowLevel};
    window.with_winit_window(|w| {
        w.set_window_level(if enabled {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        })
    });
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            enable_custom_frame(hwnd);
        }
    }
}
pub fn launcher_window(window: &slint::Window) {
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            let _ = SetPropW(hwnd, w!("Diascia.Launcher"), Some(HANDLE(1usize as _)));
        }
    }
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::platform::windows::WindowExtWindows;
    window.with_winit_window(|w| w.set_skip_taskbar(true));
    decorate(window);
}
/// Idempotent: safe on the first native event, and after a recreated HWND.
pub fn ensure_launcher(window: &slint::Window) {
    if native_handle(window)
        .is_some_and(|hwnd| unsafe { GetPropW(hwnd, w!("Diascia.Launcher")).is_invalid() })
    {
        launcher_window(window);
    }
}
/// `ensure_launcher` for a per-event path. Resolving the HWND through winit costs more than the
/// property read that guards it, so the handle is cached — but not the answer. A stale handle
/// fails `IsWindow`, falls back to a fresh lookup, and a recreated window is re-subclassed.
#[derive(Default)]
pub struct LauncherProbe(Option<HWND>);
impl LauncherProbe {
    pub fn ensure(&mut self, window: &slint::Window) {
        let cached = self
            .0
            .filter(|hwnd| unsafe { IsWindow(Some(*hwnd)).as_bool() });
        let hwnd = match cached {
            Some(hwnd) => hwnd,
            None => {
                let Some(hwnd) = native_handle(window) else {
                    return;
                };
                self.0 = Some(hwnd);
                hwnd
            }
        };
        unsafe {
            if GetPropW(hwnd, w!("Diascia.Launcher")).is_invalid() {
                launcher_window(window);
            }
        }
    }
}
pub fn geometry(
    window: &slint::Window,
) -> Option<(crate::windowing::Rect, crate::windowing::Rect)> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|w| {
            let RawWindowHandle::Win32(h) = w.window_handle().ok()?.as_raw() else {
                return None;
            };
            unsafe {
                let hwnd = HWND(h.hwnd.get() as _);
                let mut r = RECT::default();
                GetWindowRect(hwnd, &mut r).ok()?;
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                    .as_bool()
                {
                    return None;
                }
                let cv = |r: RECT| crate::windowing::Rect {
                    x: r.left,
                    y: r.top,
                    width: r.right - r.left,
                    height: r.bottom - r.top,
                };
                Some((cv(r), cv(info.rcWork)))
            }
        })
        .flatten()
}
pub fn app_has_focus() -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}
pub fn window_active(window: &slint::Window) -> bool {
    native_handle(window).is_some_and(|hwnd| unsafe { GetForegroundWindow() == hwnd })
}
pub fn set_autostart(enabled: bool, data_dir: &Path) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| e.to_string())?;
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&format!(
                "\"{}\" --data-dir \"{}\"",
                exe.display(),
                data_dir.display()
            ));
            let bytes =
                std::slice::from_raw_parts(command.as_ptr() as *const u8, command.len() * 2);
            RegSetValueExW(key, w!("Diascia"), Some(0), REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| e.to_string())
        } else {
            let code = RegDeleteValueW(key, w!("Diascia"));
            if code == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                code.ok().map_err(|e| e.to_string())
            }
        };
        let _ = RegCloseKey(key);
        result
    }
}
/// Extract the Windows Shell icon (including shortcut resolution) and save PNG pixels.
pub fn shell_icon(path: &Path) -> Vec<u8> {
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    unsafe {
        let mut info = SHFILEINFOW::default();
        let name = wide(&path.to_string_lossy().replace('/', "\\"));
        if SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        ) == 0
        {
            return vec![];
        }
        icon_by_index(info.iIcon)
    }
}

/// Packaged applications have no file system path: their icon is served by the AppsFolder
/// namespace, so the display name is parsed into an item id and looked up in the same image list.
pub fn app_icon(aumid: &str) -> Vec<u8> {
    if !aumid.contains('!') || aumid.contains('\\') {
        return Vec::new();
    }
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    unsafe {
        let name = wide(&format!("shell:AppsFolder\\{aumid}"));
        let mut id: *mut Common::ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut id, 0, None).is_err() {
            return Vec::new();
        }
        let _item = TaskMemIdl(id);
        let mut info = SHFILEINFOW::default();
        if SHGetFileInfoW(
            PCWSTR(id as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_SYSICONINDEX,
        ) == 0
        {
            return Vec::new();
        }
        icon_by_index(info.iIcon)
    }
}

/// Single dispatch used by every icon call site: a web page is asked for its own icon, a packaged
/// app is resolved through the Shell namespace, and everything else is a path on disk.
pub fn icon_for(target: &Target, base: &Path) -> Vec<u8> {
    match target {
        Target::Url { url } => web_icon::fetch(url),
        Target::App { aumid } => app_icon(aumid),
        other => shell_icon(&crate::paths::absolute(other.value(), base)),
    }
}

/// Draw one system image-list entry onto a transparent square and encode it as PNG.
unsafe fn icon_by_index(index: i32) -> Vec<u8> {
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    unsafe {
        let (icon, size) = [
            (SHIL_JUMBO as i32, 256),
            (SHIL_EXTRALARGE as i32, 48),
            (SHIL_LARGE as i32, 32),
        ]
        .into_iter()
        .find_map(|(list, size)| {
            SHGetImageList::<IImageList>(list)
                .ok()
                .and_then(|images| images.GetIcon(index, ILD_TRANSPARENT.0).ok())
                .map(|icon| (icon, size))
        })
        .unwrap_or((
            windows::Win32::UI::WindowsAndMessaging::HICON::default(),
            32,
        ));
        if icon.is_invalid() {
            return vec![];
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            let _ = DestroyIcon(icon);
            return vec![];
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            let _ = DestroyIcon(icon);
            return vec![];
        };
        let old = SelectObject(dc, bitmap.into());
        std::ptr::write_bytes(bits, 0, (size * size * 4) as usize);
        let result = DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL);
        let mut bytes =
            std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize).to_vec();
        // Legacy mask-based icons have no alpha. Drawing on white and black
        // recovers coverage, including opaque black pixels (not a color key).
        if bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            std::ptr::write_bytes(bits, 255, (size * size * 4) as usize);
            if DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL).is_ok() {
                let white =
                    std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize);
                for (p, w) in bytes
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(white.as_chunks::<4>().0)
                {
                    p[3] = 255 - w[0].saturating_sub(p[0]);
                }
            }
        }
        let _ = SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        let _ = DestroyIcon(icon);
        if result.is_err() {
            return vec![];
        }
        for p in bytes.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 / a).min(255)) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(size as u32, size as u32, bytes)
            .and_then(|i| crate::icons::encode(i).ok())
            .unwrap_or_default()
    }
}

pub fn close_menu_focus(window: &slint::Window) {
    focus(window);
}

#[path = "fluent_menu.rs"]
mod fluent_menu;
#[path = "shell_actions.rs"]
mod shell_actions;
#[path = "web_icon.rs"]
mod web_icon;
pub use shell_actions::{create_shortcut_in, desktop_shortcut, shell_context_menu};
pub fn show_menu(
    window: &slint::Window,
    items: &[crate::menus::MenuItem],
    background: slint::Color,
) -> Option<u32> {
    fluent_menu::show(window, items, background)
}

pub fn monitor_name(window: &slint::Window) -> String {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|w| {
            w.current_monitor()
                .and_then(|m| m.name())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}
pub fn restore_saved(window: &slint::Window, state: &crate::session::WindowState) {
    use slint::winit_030::WinitWindowAccessor;
    if !state.valid_geometry() {
        return;
    }
    let monitor = window.with_winit_window(|w| {
        w.available_monitors()
            .find(|m| m.name().as_deref() == Some(&state.monitor))
    });
    let monitor = monitor.flatten();
    let target_scale = monitor
        .as_ref()
        .map_or(window.scale_factor(), |m| m.scale_factor() as f32);
    crate::metrics::record("restore_monitor_scale", target_scale as f64);
    crate::metrics::record("restore_window_scale", window.scale_factor() as f64);
    let origin = monitor.map(|m| m.position());
    let work = origin
        .and_then(|p| work_area_at((p.x, p.y)))
        .or_else(|| work_area_at((state.x, state.y)));
    if let Some(work) = work {
        let scale = target_scale;
        let width = (state.width * scale).round().min(work.width as f32) as i32;
        let height = (state.height * scale).round().min(work.height as f32) as i32;
        let mut x = state.x.clamp(work.x, work.x + (work.width - width).max(0));
        let mut y = state
            .y
            .clamp(work.y, work.y + (work.height - height).max(0));
        match state.edge {
            Some(0) => x = work.x,
            Some(1) => x = work.x + work.width - width,
            Some(2) => y = work.y,
            _ => {}
        }
        window.set_size(slint::PhysicalSize::new(width as u32, height as u32));
        window.set_position(slint::PhysicalPosition::new(x, y));
    }
}

#[path = "drop_target.rs"]
pub mod drop_target;

#[cfg(test)]
mod protection_tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn fullscreen_rect_matches_exact_and_within_tolerance() {
        let screen = rect(0, 0, 2560, 1440);
        assert!(fullscreen_rect(rect(0, 0, 2560, 1440), screen, 8));
        assert!(fullscreen_rect(rect(-4, 3, 2564, 1436), screen, 8));
        assert!(fullscreen_rect(rect(-16, 16, 2576, 1424), screen, 24));
    }

    #[test]
    fn fullscreen_rect_rejects_small_gaps_and_overruns() {
        let screen = rect(0, 0, 2560, 1440);
        assert!(!fullscreen_rect(rect(-9, 0, 2560, 1440), screen, 8));
        assert!(!fullscreen_rect(rect(0, 40, 2560, 1440), screen, 24));
        assert!(!fullscreen_rect(rect(-100, 0, 2660, 1440), screen, 24));
        assert!(!fullscreen_rect(rect(0, 0, 1280, 720), screen, 8));
    }

    #[test]
    fn fullscreen_rect_uses_secondary_monitor_origin() {
        let screen = rect(-1920, 0, 0, 1080);
        assert!(fullscreen_rect(rect(-1920, 0, 0, 1080), screen, 8));
        assert!(!fullscreen_rect(rect(0, 0, 1920, 1080), screen, 8));
    }
}

#[cfg(test)]
mod instance_tests {
    use super::*;
    #[test]
    fn secondary_event_is_delivered_without_polling_and_waiter_stops() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = Instance::acquire(dir.path()).unwrap().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        instance.listen(Arc::new(move |code, _| {
            let _ = sender.send(code);
        }));
        assert!(Instance::acquire(dir.path()).unwrap().is_none());
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            7
        );
        drop(instance);
        assert!(Instance::acquire(dir.path()).unwrap().is_some());
    }
}
#[cfg(test)]
mod app_list_tests {
    use super::*;
    #[test]
    fn only_app_ids_survive_the_apps_folder_filter() {
        assert_eq!(
            app_option("计算器", r"shell:AppsFolder\Package_1.0_x64!App"),
            Some(("Package_1.0_x64!App".into(), "计算器".into()))
        );
        assert_eq!(
            app_option("缺少入口", r"shell:AppsFolder\Package_1.0_x64"),
            None
        );
        assert_eq!(app_option("文件夹自身", r"shell:AppsFolder"), None);
        assert_eq!(app_option("  ", r"shell:AppsFolder\Package!App"), None);
        assert_eq!(
            app_option("桌面", r"C:\Users\Public\Desktop"),
            None,
            "filesystem children are not app entries"
        );
    }
    #[test]
    fn apps_folder_enumeration_stays_parseable_or_returns_nothing() {
        // How many packaged apps exist is machine-dependent; what must hold either way is that
        // every returned id is accepted by the launch path that will consume it.
        let apps = installed_apps();
        assert!(!apps.is_empty(), "expected packaged apps on this machine");
        for (aumid, name) in apps {
            assert!(aumid.contains('!'), "{aumid}");
            assert!(!name.trim().is_empty());
            assert!(
                Target::App {
                    aumid: aumid.clone()
                }
                .validate()
                .is_ok(),
                "{aumid}"
            );
        }
    }
    /// AppsFolder icons are what tell two packaged apps apart in the grid, so a silent fallback to
    /// the generic placeholder has to fail here instead of looking like a missing icon in the UI.
    #[test]
    fn apps_folder_icons_are_extracted_per_app() {
        let apps = installed_apps();
        let mut sizes = std::collections::HashSet::new();
        for (aumid, _) in apps.iter().take(6) {
            let png = app_icon(aumid);
            let image = image::load_from_memory(&png)
                .unwrap_or_else(|e| panic!("{aumid}: {} bytes are not an icon ({e})", png.len()));
            assert!(image.width() >= 32, "{aumid} {}", image.width());
            assert!(image.height() >= 32, "{aumid} {}", image.height());
            sizes.insert(png.len());
        }
        assert!(
            sizes.len() > 1,
            "every app resolved to the same image: generic fallback"
        );
        assert!(app_icon("no-entry-separator").is_empty());
    }
}
