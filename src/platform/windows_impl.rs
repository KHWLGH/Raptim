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
        System::{Com::*, Threading::*},
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

/// What one `WaitForMultipleObjects` return means for the summon waiter. The handles are ordered
/// `[stop, event]`, so only index 0 ends the thread. Everything that is not index 1 — `WAIT_FAILED`
/// included — is retried: a waiter that exits on a transient failure takes second-instance summoning
/// down with it for the rest of the run, and nothing re-arms it.
#[derive(PartialEq, Debug)]
enum Wait {
    Stop,
    Summon,
    Retry,
}
fn classify_wait(result: WAIT_EVENT) -> Wait {
    if result == WAIT_OBJECT_0 {
        Wait::Stop
    } else if result == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) {
        Wait::Summon
    } else {
        Wait::Retry
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
        let prefix = format!("Local\\Raptim-{:016x}", hasher.finish());
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
                match classify_wait(WaitForMultipleObjects(&handles, false, INFINITE)) {
                    Wait::Stop => break,
                    Wait::Summon => notify(7, String::new()),
                    // Back off rather than spin: if a call is failing it will keep failing, and a
                    // hot loop here costs more than the summon it is waiting for.
                    Wait::Retry => thread::sleep(std::time::Duration::from_millis(50)),
                }
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

/// When a program entry leaves `working_dir` empty, default to its own parent directory (if it
/// exists) rather than letting `ShellExecuteExW` inherit Raptim's current working directory.
pub fn effective_working_dir(path: &str, working_dir: &str) -> String {
    let trimmed = working_dir.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty() && parent.is_dir())
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default()
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
                path,
                arguments,
                working_dir,
                elevated,
                show,
            } => (
                wide(arguments),
                wide(&effective_working_dir(path, working_dir)),
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
    /// The last `(shortcut label, enabled)` pair the worker confirmed. This is a log of what was
    /// *requested*, not a belief about what the OS holds: `configure` clears it whenever the reply is
    /// an error or never arrives, so the worst it can do is cause one redundant re-send. It used to
    /// be the assumed registration, and a request that timed out left it behind — after which
    /// `set_enabled(false)` compared against the stale value and could skip the unregistration the
    /// OS still needed, leaving a "disabled" hotkey globally grabbed.
    requested: Option<(String, bool)>,
    sender: mpsc::Sender<HotkeyRequest>,
}
impl Hotkey {
    pub fn new(notify: Notify, enabled: bool, shortcut: &str) -> Result<Self, String> {
        // Parsed here so an unreadable shortcut saved in the database still fails startup where it
        // used to, rather than only surfacing as a toast.
        crate::hotkey::parse(shortcut)?;
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
            requested: None,
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
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Ok(())) => {
                self.requested = Some((shortcut.to_string(), enabled));
                Ok(())
            }
            // The worker kept the previous binding.
            Ok(Err(text)) => {
                self.requested = None;
                Err(text)
            }
            Err(e) => {
                // The request stays queued and is applied whenever the pump reaches it, so this side
                // must not remember an answer it never got: clearing the log makes the next call
                // re-send instead of assuming the old pair still holds.
                self.requested = None;
                Err(e.to_string())
            }
        }
    }
    /// Re-applies `shortcut` unless the worker already confirmed exactly that pair. Called from the
    /// protection tick, so the confirmed pair is what keeps it off the worker's queue every 500 ms.
    pub fn set_enabled(&mut self, shortcut: &str, enabled: bool) {
        if self
            .requested
            .as_ref()
            .is_some_and(|(label, on)| label == shortcut && *on == enabled)
        {
            return;
        }
        let _ = self.configure(shortcut, enabled);
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
    if matches!(
        message,
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_THEMECHANGED
    ) {
        FOREGROUND_NOTIFY.with(|slot| {
            if let Some(notify) = slot.borrow().as_ref() {
                notify(
                    if message == WM_DISPLAYCHANGE { 17 } else { 21 },
                    String::new(),
                );
            }
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

const DISPLAY_CLASS: PCWSTR = w!("Raptim.DisplayWatch");

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
        let show = MenuItem::new("显示 Raptim", true, None);
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
            pause_hide,
            pause_protection,
        })
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
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // Prefer direct foreground activation when Windows already permits it (e.g.
            // WM_HOTKEY / tray click), avoiding winit's synthetic Alt SendInput fallback.
            if GetForegroundWindow() == hwnd || SetForegroundWindow(hwnd).as_bool() {
                return;
            }
        }
    }
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|w| {
        w.focus_window();
    });
}

/// Schedule a native WM_PAINT immediately, bypassing Slint's TimerBasedFrameThrottle
/// (which otherwise sleeps one display refresh period before requesting a redraw).
pub fn request_redraw_now(window: &slint::Window) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|w| {
            w.request_redraw();
            true
        })
        .unwrap_or(false)
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
pub fn set_bounds(window: &slint::Window, rect: crate::windowing::Rect) {
    let width = rect.width.max(1);
    let height = rect.height.max(1);
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.x,
                rect.y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        return;
    }
    window.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    window.set_position(slint::PhysicalPosition::new(rect.x, rect.y));
}
pub fn set_size_no_activate(window: &slint::Window, width: i32, height: i32) {
    let width = width.max(1);
    let height = height.max(1);
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        return;
    }
    window.set_size(slint::PhysicalSize::new(width as u32, height as u32));
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
/// Frame bookkeeping, kept in window properties. A property dies with its HWND, so a recreated
/// window invalidates every "already applied" answer by itself and no global cache can go stale.
/// `w!` needs a literal, so these take the name at call time.
unsafe fn prop_value(hwnd: HWND, name: &str) -> usize {
    let name = wide(name);
    unsafe { GetPropW(hwnd, PCWSTR(name.as_ptr())).0 as usize }
}
unsafe fn set_prop_value(hwnd: HWND, name: &str, value: usize) {
    let name = wide(name);
    unsafe {
        let _ = SetPropW(hwnd, PCWSTR(name.as_ptr()), Some(HANDLE(value as _)));
    }
}
/// Take the window shape back from our own region and hand it to the compositor. Only ever
/// touches a window we actually gave a region to: `SetWindowRgn` rebuilds the frame, so calling it
/// on a window that already has none is what makes corners and shadows jump.
unsafe fn clear_region(hwnd: HWND) {
    unsafe {
        if prop_value(hwnd, "Raptim.Region") != 0 {
            set_prop_value(hwnd, "Raptim.Region", 0);
            let _ = SetWindowRgn(hwnd, None, true);
        }
    }
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
            RemovePropW(hwnd, w!("Raptim.Drag")).unwrap_or_default().0 as *mut LauncherDrag;
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
        if SetPropW(hwnd, w!("Raptim.Drag"), Some(HANDLE(pointer as _))).is_err() {
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
        let launcher = !GetPropW(hwnd, w!("Raptim.Launcher")).is_invalid();
        if launcher {
            match message {
                WM_SYSCOMMAND if matches!(wparam.0 as u32 & 0xfff0, SC_MAXIMIZE | SC_MOVE) => {
                    return LRESULT(0);
                }
                WM_NCLBUTTONDBLCLK if wparam.0 as u32 == HTCAPTION => return LRESULT(0),
                WM_MOUSEMOVE => {
                    let pointer = GetPropW(hwnd, w!("Raptim.Drag")).0 as *const LauncherDrag;
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
                    if !GetPropW(hwnd, w!("Raptim.Drag")).is_invalid() {
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
            WM_ERASEBKGND => LRESULT(1),
            WM_STYLECHANGING if lparam.0 != 0 => {
                // winit's set_visible / WindowFlags::apply_diff rewrites GWL_STYLE and
                // GWL_EXSTYLE from its own flag cache (restoring WS_CAPTION and dropping
                // WS_EX_TOOLWINDOW). Filter those writes before Windows applies them so
                // showing or hiding a window never triggers a non-client frame/corner pop.
                let styles = &mut *(lparam.0 as *mut STYLESTRUCT);
                if wparam.0 as i32 == GWL_STYLE.0 {
                    let mut style = (styles.styleNew & !WS_CAPTION.0) | WS_THICKFRAME.0;
                    if launcher {
                        style &= !WS_MAXIMIZEBOX.0;
                    }
                    styles.styleNew = style;
                } else if wparam.0 as i32 == GWL_EXSTYLE.0 {
                    styles.styleNew = (styles.styleNew & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0;
                }
                LRESULT(0)
            }
            WM_NCDESTROY => {
                clear_launcher_drag(hwnd);
                let _ = RemoveWindowSubclass(hwnd, Some(frame_proc), id);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, message, wparam, lparam),
        }
    }
}
/// Frame decoration. Called from every show, reveal, un-dock and resize path, so nothing here may
/// assume it is the first call: each write below rebuilds DWM's frame, and a rebuild while the
/// window is sliding is the corner-and-shadow pop this used to show. Every one of them is now
/// guarded by the state already applied to this HWND.
pub fn decorate(window: &slint::Window) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    use slint::winit_030::{WinitWindowAccessor, winit::window::WindowButtons};
    unsafe {
        // Read before the remove: a work-area clip leaves DONOTROUND and a region behind, and
        // 2 is the only marker that says so.
        let undoing_clip = prop_value(hwnd, "Raptim.EdgeClip") == 2;
        let _ = RemovePropW(hwnd, w!("Raptim.EdgeClip"));
        enable_custom_frame(hwnd);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let tool = (ex & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize;
        if ex != tool {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, tool);
        }
        if prop_value(hwnd, "Raptim.Buttons") == 0 {
            set_prop_value(hwnd, "Raptim.Buttons", 1);
            let already_close_only = window
                .with_winit_window(|w| w.enabled_buttons() == WindowButtons::CLOSE)
                .unwrap_or(false);
            if !already_close_only {
                window.with_winit_window(|w| {
                    // 窗口隐藏到托盘/贴边即为关闭语义；最小化会让跳过任务栏的窗口无法找回。
                    // winit writes GWL_STYLE and re-frames the window to apply this.
                    w.set_enabled_buttons(WindowButtons::CLOSE);
                });
            }
        }
        // Only seed Raptim.Dark on first decoration so explicit Light/Dark overrides via
        // `apply_window_theme` are never reverted by `winit::Window::theme()`.
        if prop_value(hwnd, "Raptim.Dark") == 0 {
            let dark = system_dark_mode();
            set_prop_value(hwnd, "Raptim.Dark", dark as usize + 1);
            let immersive = windows::core::BOOL::from(dark);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &immersive as *const _ as _,
                4,
            );
        }
        // 1: the compositor rounds this window. 2: it cannot, so our own region does.
        let frame = prop_value(hwnd, "Raptim.Frame");
        if frame == 0 || undoing_clip || frame == 2 {
            let corner = DWMWCP_ROUND;
            let rounded = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as _,
                4,
            )
            .is_ok();
            let policy = DWMNCRP_ENABLED;
            let _ =
                DwmSetWindowAttribute(hwnd, DWMWA_NCRENDERING_POLICY, &policy as *const _ as _, 4);
            set_prop_value(hwnd, "Raptim.ShadowMuted", 0);
            // Disable DWM's show/hide zoom transition so re-summoning maps immediately
            // with DWMWCP_ROUND intact instead of animating a square snapshot and snapping
            // to rounded corners after the transition completes.
            let no_transitions = windows::core::BOOL::from(true);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &no_transitions as *const _ as _,
                4,
            );
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
            refresh_border(window);
            if rounded {
                // Let the compositor anti-alias corners. An HRGN here disables DWM
                // rounding and creates visibly jagged edges, especially at high DPI.
                clear_region(hwnd);
                set_prop_value(hwnd, "Raptim.Frame", 1);
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
                set_prop_value(hwnd, "Raptim.Region", 1);
                if SetWindowRgn(hwnd, Some(region), true) == 0 {
                    let _ = DeleteObject(region.into());
                }
                set_prop_value(hwnd, "Raptim.Frame", 2);
            }
        }
    }
}
/// Skip the taskbar for a Slint window. Checked against the live style bits rather than remembered:
/// winit rebuilds both style words from its own idea of the window every time it applies a style,
/// so a "this was already done" note could go stale and leave the launcher in the taskbar.
pub fn skip_taskbar(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::platform::windows::WindowExtWindows;
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    let hidden = unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        ex & WS_EX_TOOLWINDOW.0 as isize != 0 && ex & WS_EX_APPWINDOW.0 as isize == 0
    };
    if hidden {
        return;
    }
    window.with_winit_window(|w| w.set_skip_taskbar(true));
}
/// Re-apply the decoration after a resize, a scale-factor change or a theme change.
pub fn refresh_geometry(window: &slint::Window) {
    decorate(window);
}
unsafe fn enable_custom_frame(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(frame_proc), 0xD1A5, 0);
        let old = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let mut style = (old & !(WS_CAPTION.0 as isize)) | WS_THICKFRAME.0 as isize;
        if !GetPropW(hwnd, w!("Raptim.Launcher")).is_invalid() {
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
    let retained_clip = unsafe { GetPropW(hwnd, w!("Raptim.EdgeClip")).0 as usize == 2 };
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
        let mut old = RECT::default();
        let moved = GetWindowRect(hwnd, &mut old).is_err() || (old.left, old.top) != position;
        if !clip {
            let _ = SetPropW(hwnd, w!("Raptim.EdgeClip"), Some(HANDLE(1usize as _)));
        }
        if moved {
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
    }
    if clip {
        clip_to_work_area(window, work);
    }
}
/// Complete a docked slide without rebuilding its frame/corners on the last tick.
pub fn finish_edge_motion(window: &slint::Window) {
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            if GetPropW(hwnd, w!("Raptim.EdgeClip")).0 as usize == 1 {
                let _ = RemovePropW(hwnd, w!("Raptim.EdgeClip"));
            }
        }
    }
}
/// Suppresses DWM shadows at rest on a collapsed docked window (either cloaking a
/// 0-pixel hide or disabling NC rendering when only a thin strip remains visible).
pub fn set_collapsed_appearance(
    window: &slint::Window,
    appearance: crate::docking::CollapsedAppearance,
) {
    use crate::docking::CollapsedAppearance;
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        let want_cloaked = appearance == CollapsedAppearance::Cloaked;
        let mut want_muted = appearance == CollapsedAppearance::StripOnly;
        let is_cloaked = prop_value(hwnd, "Raptim.Cloaked") == 1;
        if want_cloaked && !is_cloaked {
            let cloak = windows::core::BOOL::from(true);
            if DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const _ as _, 4).is_ok() {
                set_prop_value(hwnd, "Raptim.Cloaked", 1);
            } else {
                want_muted = true;
            }
        }

        let is_muted = prop_value(hwnd, "Raptim.ShadowMuted") == 1;
        let edge_clipped = prop_value(hwnd, "Raptim.EdgeClip") == 2;
        let region_fallback = prop_value(hwnd, "Raptim.Frame") == 2;
        if want_muted && !is_muted {
            if !edge_clipped {
                let corner = DWMWCP_DONOTROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner as *const _ as _,
                    4,
                );
                let policy = DWMNCRP_DISABLED;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    &policy as *const _ as _,
                    4,
                );
                set_prop_value(hwnd, "Raptim.ShadowMuted", 1);
            }
        } else if !want_muted && is_muted {
            set_prop_value(hwnd, "Raptim.ShadowMuted", 0);
            if !edge_clipped && !region_fallback {
                let corner = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner as *const _ as _,
                    4,
                );
                let policy = DWMNCRP_ENABLED;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    &policy as *const _ as _,
                    4,
                );
                let border = COLORREF(0xfffffffe);
                let _ =
                    DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &border as *const _ as _, 4);
                let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
            }
        }

        if !want_cloaked && prop_value(hwnd, "Raptim.Cloaked") == 1 {
            let cloak = windows::core::BOOL::from(false);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const _ as _, 4);
            set_prop_value(hwnd, "Raptim.Cloaked", 0);
            crate::windowing::invalidate(window);
            request_redraw_now(window);
        }
    }
}
/// Native clip prevents a sliding window leaking onto an adjacent display.
pub fn clip_to_work_area(window: &slint::Window, work: crate::windowing::Rect) {
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        let first = GetPropW(hwnd, w!("Raptim.EdgeClip")).0 as usize != 2;
        let _ = SetPropW(hwnd, w!("Raptim.EdgeClip"), Some(HANDLE(2usize as _)));
        // This path installs a region outside `decorate`, and `clear_region` only hands the shape
        // back to the compositor if it knows one is installed.
        set_prop_value(hwnd, "Raptim.Region", 1);
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
        if GetPropW(hwnd, w!("Raptim.Alpha")).0 as usize == alpha + 1 {
            return;
        }
        let _ = SetPropW(hwnd, w!("Raptim.Alpha"), Some(HANDLE((alpha + 1) as _)));
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if opacity >= 1.0 {
            if style & WS_EX_LAYERED.0 as isize != 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED.0 as isize));
                if window.is_visible() {
                    window.request_redraw();
                }
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
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if (ex & WS_EX_TOPMOST.0 as isize != 0) == enabled {
            // Already in that band - the common answer on a summon path. winit's `set_window_level`
            // reaches the same place by rewriting both style words from its own state, which drops
            // `WS_EX_TOOLWINDOW` and gives DWM a reason to redraw the frame under a window that is
            // mid-slide, so the band is set directly here.
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            Some(if enabled {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        enable_custom_frame(hwnd);
    }
}
pub fn launcher_window(window: &slint::Window) {
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            let _ = SetPropW(hwnd, w!("Raptim.Launcher"), Some(HANDLE(1usize as _)));
        }
    }
    skip_taskbar(window);
    decorate(window);
}
/// Idempotent: safe on the first native event, and after a recreated HWND.
pub fn ensure_launcher(window: &slint::Window) {
    if native_handle(window)
        .is_some_and(|hwnd| unsafe { GetPropW(hwnd, w!("Raptim.Launcher")).is_invalid() })
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
            if GetPropW(hwnd, w!("Raptim.Launcher")).is_invalid() {
                launcher_window(window);
            }
        }
    }
}
/// Reads Windows' app light/dark preference directly from the registry so collapsed
/// or cloaked windows never depend on DWM/winit window-visibility state.
pub fn system_dark_mode() -> bool {
    use windows::Win32::System::Registry::*;
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && value == 0
}

pub fn resolve_dark_mode(theme_mode: u8) -> bool {
    match theme_mode {
        1 => false,
        2 => true,
        _ => system_dark_mode(),
    }
}

pub fn apply_window_theme(window: &slint::Window, theme_mode: u8, force_redraw: bool) -> bool {
    let dark = resolve_dark_mode(theme_mode);
    use slint::winit_030::{WinitWindowAccessor, winit::window::Theme};
    let scheme = if dark {
        i_slint_core::items::ColorScheme::Dark
    } else {
        i_slint_core::items::ColorScheme::Light
    };
    let inner = i_slint_core::window::WindowInner::from_pub(window);
    let scheme_changed = inner.context().color_scheme(None) != scheme;
    inner.context().set_color_scheme(scheme);
    let mut hwnd_changed = false;
    if let Some(hwnd) = native_handle(window) {
        unsafe {
            if prop_value(hwnd, "Raptim.Dark") != dark as usize + 1 {
                set_prop_value(hwnd, "Raptim.Dark", dark as usize + 1);
                hwnd_changed = true;
                let immersive = windows::core::BOOL::from(dark);
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &immersive as *const _ as _,
                    4,
                );
            }
        }
    }
    if scheme_changed || hwnd_changed || force_redraw {
        let _ = window.with_winit_window(|w| {
            w.set_theme(Some(if dark { Theme::Dark } else { Theme::Light }));
        });
        crate::windowing::invalidate(window);
        request_redraw_now(window);
    }
    scheme_changed || hwnd_changed
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
pub fn autostart_command(exe: &Path, data_dir: &Path) -> String {
    let clean_dir = crate::paths::normal_path(data_dir);
    format!(
        "\"{}\" --data-dir \"{}\" --silent",
        exe.display(),
        clean_dir.display()
    )
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
            let command = wide(&autostart_command(&exe, data_dir));
            let bytes =
                std::slice::from_raw_parts(command.as_ptr() as *const u8, command.len() * 2);
            RegSetValueExW(key, w!("Raptim"), Some(0), REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| e.to_string())
        } else {
            let code = RegDeleteValueW(key, w!("Raptim"));
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
/// Check whether the Windows `Run` entry and Task Manager approval match the saved preference.
pub fn autostart_mismatch(data_dir: &Path, expected_enabled: bool) -> Option<String> {
    use windows::Win32::System::Registry::*;
    if !expected_enabled {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let expected = autostart_command(&exe, data_dir);
    unsafe {
        let mut run_key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut run_key,
        )
        .is_err()
        {
            return Some("开机启动项未写入系统注册表，请重新开关生效".into());
        }
        let mut reg_type = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let query = RegQueryValueExW(
            run_key,
            w!("Raptim"),
            None,
            Some(&mut reg_type),
            None,
            Some(&mut size),
        );
        let actual = if query.is_ok() && reg_type == REG_SZ && size >= 2 {
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            let mut read = size;
            if RegQueryValueExW(
                run_key,
                w!("Raptim"),
                None,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut read),
            )
            .is_ok()
            {
                let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                Some(String::from_utf16_lossy(&buf[..end]))
            } else {
                None
            }
        } else {
            None
        };
        let _ = RegCloseKey(run_key);
        let Some(actual) = actual else {
            return Some("开机启动项未写入系统注册表，请重新开关生效".into());
        };
        if !actual.trim().eq_ignore_ascii_case(expected.trim()) {
            return Some("程序路径、数据目录或静默参数已变更，请重新开关「开机启动」更新".into());
        }
        let mut approved = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut approved,
        )
        .is_ok()
        {
            let mut buf = [0u8; 16];
            let mut read = buf.len() as u32;
            let mut kind = REG_VALUE_TYPE::default();
            if RegQueryValueExW(
                approved,
                w!("Raptim"),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr()),
                Some(&mut read),
            )
            .is_ok()
                && read >= 1
                && (buf[0] & 1 == 1)
            {
                let _ = RegCloseKey(approved);
                return Some("系统任务管理器已禁用开机启动，请在任务管理器启用或重新开关".into());
            }
            let _ = RegCloseKey(approved);
        }
    }
    None
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
pub use shell_actions::{
    create_shortcut_in, desktop_shortcut, known_folder_path, resolve_lnk_file, resolve_url_file,
    shell_context_menu,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredApp {
    pub name: String,
    pub detail: String,
    pub target: Target,
    pub icon_source: Option<std::path::PathBuf>,
}

fn is_noise_shortcut(name: &str, target: &Target) -> bool {
    let low_name = name.to_lowercase();
    let low_val = target.value().to_lowercase();
    [
        "uninstall",
        "uninst",
        "卸载",
        "readme",
        "读我",
        "帮助",
        "manual",
        "license",
        "用户协议",
    ]
    .iter()
    .any(|kw| low_name.contains(kw))
        || low_val.contains("unins0")
        || low_val.contains("uninstall")
}

fn collect_shortcuts_in(
    dir: &Path,
    depth: u8,
    self_exe: &str,
    seen_keys: &mut std::collections::HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() && !ft.is_symlink() {
            collect_shortcuts_in(&path, depth + 1, self_exe, seen_keys, out);
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "lnk" | "url") {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let target = if ext == "url" {
            match resolve_url_file(&path) {
                Some(url) => Target::Url { url },
                None => Target::Shortcut {
                    path: path.to_string_lossy().into_owned(),
                    show: ShowMode::Normal,
                },
            }
        } else {
            resolve_lnk_file(&path).unwrap_or_else(|| Target::Shortcut {
                path: path.to_string_lossy().into_owned(),
                show: ShowMode::Normal,
            })
        };
        if target.validate().is_err() || is_noise_shortcut(&name, &target) {
            continue;
        }
        if !self_exe.is_empty()
            && target
                .value()
                .replace('/', "\\")
                .eq_ignore_ascii_case(self_exe)
        {
            continue;
        }
        if seen_keys.insert(target.duplicate_key()) {
            let detail = target.value().to_string();
            out.push(DiscoveredApp {
                name,
                detail,
                target,
                icon_source: Some(path),
            });
        }
    }
}

/// Discover installed applications from Start Menu shortcuts and packaged AppsFolder entries.
pub fn discover_installed_entries() -> Vec<DiscoveredApp> {
    let self_exe = std::env::current_exe()
        .map(|p| {
            crate::paths::normal_path(&p)
                .to_string_lossy()
                .replace('/', "\\")
        })
        .unwrap_or_default();
    let mut seen_keys = std::collections::HashSet::new();
    let mut out = Vec::new();
    for folder in [&FOLDERID_CommonPrograms, &FOLDERID_Programs] {
        if let Ok(dir) = known_folder_path(folder) {
            collect_shortcuts_in(&dir, 0, &self_exe, &mut seen_keys, &mut out);
        }
    }
    let seen_names: std::collections::HashSet<String> =
        out.iter().map(|d| d.name.to_lowercase()).collect();
    for (aumid, name) in installed_apps() {
        let target = Target::App {
            aumid: aumid.clone(),
        };
        if seen_names.contains(&name.to_lowercase()) || target.validate().is_err() {
            continue;
        }
        if seen_keys.insert(target.duplicate_key()) {
            out.push(DiscoveredApp {
                name,
                detail: aumid,
                target,
                icon_source: None,
            });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}
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
        // Move to the destination monitor first so winit adjusts DPI and minimum-size constraints
        // before resizing across monitors with different scale factors.
        window.set_position(slint::PhysicalPosition::new(x, y));
        window.set_size(slint::PhysicalSize::new(width as u32, height as u32));
        window.set_position(slint::PhysicalPosition::new(x, y));
    }
}

#[path = "drop_target.rs"]
pub mod drop_target;

#[cfg(test)]
mod tests {
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
    fn working_dir_fallback_and_fullscreen_rect_matching() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(
            effective_working_dir(&exe.to_string_lossy(), ""),
            dir.path().to_string_lossy()
        );
        assert_eq!(
            effective_working_dir(&exe.to_string_lossy(), "  C:\\Custom  "),
            "C:\\Custom"
        );

        let screen = rect(0, 0, 2560, 1440);
        assert!(fullscreen_rect(rect(0, 0, 2560, 1440), screen, 8));
        assert!(fullscreen_rect(rect(-4, 3, 2564, 1436), screen, 8));
        assert!(!fullscreen_rect(rect(0, 40, 2560, 1440), screen, 24));
        assert!(!fullscreen_rect(rect(0, 0, 1280, 720), screen, 8));
    }

    #[test]
    fn app_option_and_wait_classification() {
        assert_eq!(
            app_option("计算器", r"shell:AppsFolder\Package_1.0_x64!App"),
            Some(("Package_1.0_x64!App".into(), "计算器".into()))
        );
        assert_eq!(
            app_option("缺少入口", r"shell:AppsFolder\Package_1.0_x64"),
            None
        );
        assert_eq!(app_option("桌面", r"C:\Users\Public\Desktop"), None);

        assert_eq!(classify_wait(WAIT_OBJECT_0), Wait::Stop);
        assert_eq!(classify_wait(WAIT_EVENT(WAIT_OBJECT_0.0 + 1)), Wait::Summon);
        assert_eq!(classify_wait(WAIT_EVENT(u32::MAX)), Wait::Retry);
    }
}
