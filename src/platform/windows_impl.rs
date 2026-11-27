use crate::{
    launch::LaunchError,
    model::{Settings, ShowMode, Target},
    policy::{Notification, Protection, Signals, decide},
};
use std::{
    cell::RefCell,
    collections::HashMap,
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

/// Strongly-typed native background notifications, replacing magic integer codes `1..=21`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeSignal {
    HotkeyPressed,
    StatusMessage(String),
    TrayShow,
    TraySettings,
    TrayQuit,
    TrayRestorePosition,
    SecondaryInstanceSummon,
    FrameClockTick(u64),
    ForegroundChanged,
    TrayTogglePin,
    TrayToggleProtectionBypass,
    DisplayChanged,
    SystemThemeChanged,
}
impl NativeSignal {
    pub fn requires_ticket(&self) -> bool {
        matches!(
            self,
            Self::HotkeyPressed
                | Self::TrayShow
                | Self::TraySettings
                | Self::TrayRestorePosition
                | Self::SecondaryInstanceSummon
        )
    }
}

pub type Notify = Arc<dyn Fn(NativeSignal) + Send + Sync>;

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
                    Wait::Summon => notify(NativeSignal::SecondaryInstanceSummon),
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
    let _com = ComGuard::new().map_err(|e| {
        LaunchError::Other(format!(
            "{}{e}",
            crate::i18n::t("COM 初始化失败：", "COM initialization failed: ")
        ))
    })?;
    unsafe {
        if let Target::App { aumid } = target {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| {
                        LaunchError::Other(format!(
                            "{}{e}",
                            crate::i18n::t(
                                "无法创建应用激活器：",
                                "Failed to create app activator: "
                            )
                        ))
                    })?;
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

fn child_insert_after(parent_topmost: bool, child_topmost: bool) -> HWND {
    match (parent_topmost, child_topmost) {
        (true, _) => HWND_TOPMOST,
        (false, true) => HWND_NOTOPMOST,
        // `HWND_NOTOPMOST` is a no-op when the child is already non-topmost, so
        // `HWND_TOP` is required to raise it above its non-topmost owner.
        (false, false) => HWND_TOP,
    }
}

pub fn set_owner(child: &slint::Window, parent: &slint::Window) {
    let (Some(owner), Some(child_hwnd)) = (native_handle(parent), native_handle(child)) else {
        return;
    };
    unsafe {
        SetWindowLongPtrW(child_hwnd, GWLP_HWNDPARENT, owner.0 as isize);
        let pinned = GetWindowLongPtrW(owner, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
        let child_pinned =
            GetWindowLongPtrW(child_hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
        let _ = SetWindowPos(
            child_hwnd,
            Some(child_insert_after(pinned, child_pinned)),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsDirty {
    Full,
    Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PendingPaint {
    Partial(RECT),
    Full,
}

thread_local! {
    static OS_DIRTY: RefCell<HashMap<isize, PendingPaint>> = RefCell::new(HashMap::new());
}

fn merge_pending_rect(existing: Option<PendingPaint>, rect: RECT) -> Option<PendingPaint> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return existing;
    }
    Some(match existing {
        Some(PendingPaint::Full) => PendingPaint::Full,
        Some(PendingPaint::Partial(prev)) => PendingPaint::Partial(RECT {
            left: prev.left.min(rect.left),
            top: prev.top.min(rect.top),
            right: prev.right.max(rect.right),
            bottom: prev.bottom.max(rect.bottom),
        }),
        None => PendingPaint::Partial(rect),
    })
}

fn record_os_dirty_full(hwnd: HWND) {
    OS_DIRTY.with(|map| {
        map.borrow_mut().insert(hwnd.0 as isize, PendingPaint::Full);
    });
}

fn record_os_dirty_rect(hwnd: HWND, rect: RECT) {
    OS_DIRTY.with(|map| {
        let mut map = map.borrow_mut();
        let current = map.get(&(hwnd.0 as isize)).copied();
        if let Some(next) = merge_pending_rect(current, rect) {
            map.insert(hwnd.0 as isize, next);
        }
    });
}

fn remove_os_dirty(hwnd: HWND) {
    OS_DIRTY.with(|map| {
        map.borrow_mut().remove(&(hwnd.0 as isize));
    });
}

pub fn surface_size(window: &slint::Window) -> slint::PhysicalSize {
    use slint::winit_030::WinitWindowAccessor;
    let current = window.size();
    window
        .with_winit_window(|w| {
            let inner = w.inner_size();
            slint::PhysicalSize::new(
                current.width.max(inner.width),
                current.height.max(inner.height),
            )
        })
        .unwrap_or(current)
}

pub fn is_native_visible(window: &slint::Window) -> bool {
    native_handle(window).is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
}

pub fn surface_paintable(window: &slint::Window) -> bool {
    let Some(hwnd) = native_handle(window) else {
        return false;
    };
    let size = surface_size(window);
    let visible =
        unsafe { IsWindowVisible(hwnd).as_bool() && prop_value(hwnd, "Raptim.Cloaked") == 0 };
    visible && size.width > 0 && size.height > 0
}

pub fn take_os_dirty(window: &slint::Window) -> Option<OsDirty> {
    if !surface_paintable(window) {
        return None;
    }
    let hwnd = native_handle(window)?;
    let pending = OS_DIRTY.with(|map| map.borrow_mut().remove(&(hwnd.0 as isize)))?;
    Some(match pending {
        PendingPaint::Full => OsDirty::Full,
        PendingPaint::Partial(r) => OsDirty::Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        },
    })
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
            WM_SHOWWINDOW => {
                if wparam.0 != 0 {
                    record_os_dirty_full(hwnd);
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_WINDOWPOSCHANGED => {
                if lparam.0 != 0 {
                    let pos = &*(lparam.0 as *const WINDOWPOS);
                    if (pos.flags & SWP_SHOWWINDOW) != SET_WINDOW_POS_FLAGS(0) {
                        record_os_dirty_full(hwnd);
                    }
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_STYLECHANGED if wparam.0 as i32 == GWL_EXSTYLE.0 && lparam.0 != 0 => {
                let styles = &*(lparam.0 as *const STYLESTRUCT);
                if (styles.styleOld & WS_EX_LAYERED.0) != 0
                    && (styles.styleNew & WS_EX_LAYERED.0) == 0
                {
                    record_os_dirty_full(hwnd);
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_DWMCOMPOSITIONCHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED => {
                record_os_dirty_full(hwnd);
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
            WM_PAINT => {
                // Capture OS-initiated update regions before winit/DefWindowProc validates them
                // or turns them into RDW_INTERNALPAINT while buffered; Slint's software renderer
                // otherwise skips BitBlt when nothing inside Slint state changed.
                let mut update = RECT::default();
                if GetUpdateRect(hwnd, Some(&mut update), false).as_bool() {
                    record_os_dirty_rect(hwnd, update);
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
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
                    let mut ex = (styles.styleNew & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0;
                    let alpha = prop_value(hwnd, "Raptim.Alpha");
                    if (1..=255).contains(&alpha) {
                        ex |= WS_EX_LAYERED.0;
                    }
                    styles.styleNew = ex;
                }
                LRESULT(0)
            }
            WM_NCDESTROY => {
                remove_os_dirty(hwnd);
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
            record_os_dirty_full(hwnd);
            crate::windowing::invalidate(window);
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
        let alpha = (opacity.clamp(0., 1.) * 255.).round() as u8;
        let want_layered = alpha < 255;
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let has_layered = style & WS_EX_LAYERED.0 as isize != 0;
        if GetPropW(hwnd, w!("Raptim.Alpha")).0 as usize == usize::from(alpha) + 1
            && has_layered == want_layered
        {
            return;
        }
        let _ = SetPropW(
            hwnd,
            w!("Raptim.Alpha"),
            Some(HANDLE((usize::from(alpha) + 1) as _)),
        );
        if !want_layered {
            if has_layered {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED.0 as isize));
                record_os_dirty_full(hwnd);
                if window.is_visible() {
                    let _ = RedrawWindow(
                        Some(hwnd),
                        None,
                        None,
                        RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN,
                    );
                    crate::windowing::invalidate(window);
                }
            }
        } else {
            if !has_layered {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
            }
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
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
        return Err(format!(
            "{}{}",
            crate::i18n::t("目标已不存在：", "Target no longer exists: "),
            path.display()
        ));
    }
    unsafe {
        let name = wide(&path.to_string_lossy());
        let pidl = ILCreateFromPathW(PCWSTR(name.as_ptr()));
        if pidl.is_null() {
            return Err(crate::i18n::t(
                "无法解析目标所在位置",
                "Failed to resolve target location",
            )
            .into());
        }
        // A fully qualified PIDL handles Unicode, commas, UNC and directories,
        // without Explorer command-line parsing or incorrectly opening the directory itself.
        let result = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(|e| {
            format!(
                "{}{e}",
                crate::i18n::t("无法打开所在位置：", "Failed to open location: ")
            )
        });
        ILFree(Some(pidl));
        result
    }
}
pub fn is_topmost(window: &slint::Window) -> bool {
    native_handle(window).is_some_and(|hwnd| unsafe {
        GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0
    })
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

fn reg_string_tag(
    subkey: PCWSTR,
    name: PCWSTR,
    flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS,
) -> Option<String> {
    use windows::Win32::System::Registry::*;
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            name,
            flags,
            None,
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size < 2 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let mut read = size;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            name,
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut read),
        )
    };
    if status != ERROR_SUCCESS || read < 2 {
        return None;
    }
    let len = (read as usize / 2).min(buf.len());
    let first = buf[..len].split(|&c| c == 0).next().unwrap_or(&[]);
    let text = String::from_utf16_lossy(first).trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub fn system_locale() -> crate::i18n::Locale {
    use windows::Win32::System::Registry::{RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ};
    let tag = reg_string_tag(
        w!("Control Panel\\Desktop"),
        w!("PreferredUILanguages"),
        RRF_RT_REG_MULTI_SZ,
    )
    .or_else(|| {
        reg_string_tag(
            w!("Control Panel\\International\\UserProfile"),
            w!("Languages"),
            RRF_RT_REG_MULTI_SZ,
        )
    })
    .or_else(|| {
        reg_string_tag(
            w!("Control Panel\\International"),
            w!("LocaleName"),
            RRF_RT_REG_SZ,
        )
    });
    tag.as_deref()
        .map_or(crate::i18n::Locale::EnUs, crate::i18n::parse_locale_tag)
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

pub fn close_menu_focus(window: &slint::Window) {
    focus(window);
}

#[path = "autostart.rs"]
mod autostart;
#[path = "discovery.rs"]
mod discovery;
#[path = "drop_target.rs"]
pub mod drop_target;
#[path = "fluent_menu.rs"]
mod fluent_menu;
#[path = "frame_clock.rs"]
mod frame_clock;
#[path = "hotkey_worker.rs"]
mod hotkey_worker;
#[path = "protection.rs"]
mod protection;
#[path = "shell_actions.rs"]
mod shell_actions;
#[path = "tray.rs"]
mod tray;
#[path = "web_icon.rs"]
mod web_icon;

pub use autostart::{autostart_command, autostart_mismatch, set_autostart};
pub use discovery::{
    DiscoveredApp, app_icon, discover_installed_entries, icon_for, installed_apps, shell_icon,
};
pub use frame_clock::FrameClock;
pub use hotkey_worker::Hotkey;
pub use protection::{
    ForegroundWatch, ProtectionCache, collect_signals, notification_state, protection,
    protection_cached,
};
pub use shell_actions::{
    create_shortcut_in, desktop_shortcut, known_folder_path, resolve_lnk_file, resolve_url_file,
    shell_context_menu,
};
pub use tray::Tray;

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

#[cfg(test)]
mod tests {
    use super::*;
    use discovery::app_option;
    use protection::fullscreen_rect;

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

    #[test]
    fn merges_os_dirty_rects_without_losing_full_paint() {
        assert_eq!(merge_pending_rect(None, rect(10, 10, 10, 30)), None);
        assert_eq!(
            merge_pending_rect(None, rect(10, 20, 40, 60)),
            Some(PendingPaint::Partial(rect(10, 20, 40, 60)))
        );
        assert_eq!(
            merge_pending_rect(
                Some(PendingPaint::Partial(rect(10, 20, 40, 60))),
                rect(25, 5, 80, 50)
            ),
            Some(PendingPaint::Partial(rect(10, 5, 80, 60)))
        );
        assert_eq!(
            merge_pending_rect(Some(PendingPaint::Full), rect(10, 20, 40, 60)),
            Some(PendingPaint::Full)
        );
    }

    #[test]
    fn resolves_child_z_order_from_parent_and_child_topmost_bits() {
        assert_eq!(child_insert_after(true, false), HWND_TOPMOST);
        assert_eq!(child_insert_after(true, true), HWND_TOPMOST);
        assert_eq!(child_insert_after(false, true), HWND_NOTOPMOST);
        assert_eq!(child_insert_after(false, false), HWND_TOP);
    }
}
