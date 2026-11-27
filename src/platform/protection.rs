//! Foreground window protection (fullscreen & strict app list) and WinEvent/Display watchers.
use super::*;

/// Caches the foreground process image path so repeated polling does not
/// reopen the process handle while the window stays unchanged.
#[derive(Default)]
pub struct ProtectionCache {
    hwnd: usize,
    pid: u32,
    path: Option<String>,
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

fn notification_state() -> Notification {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(QUNS_RUNNING_D3D_FULL_SCREEN) => Notification::RunningD3dFullScreen,
        Ok(QUNS_PRESENTATION_MODE) => Notification::PresentationMode,
        Ok(QUNS_BUSY) => Notification::Busy,
        Ok(_) => Notification::Other,
        Err(_) => Notification::Unavailable,
    }
}

fn eq_wide_ascii(slice: &[u16], ascii: &str) -> bool {
    slice.len() == ascii.len()
        && slice
            .iter()
            .zip(ascii.bytes())
            .all(|(&w, b)| w == u16::from(b))
}

fn rule_matches_path(rule: &str, path: &str) -> bool {
    if rule.is_ascii() && path.is_ascii() {
        rule.len() == path.len()
            && rule.bytes().zip(path.bytes()).all(|(r, p)| {
                let r = if r == b'/' {
                    b'\\'
                } else {
                    r.to_ascii_lowercase()
                };
                r == p
            })
    } else {
        rule.replace('/', "\\").to_lowercase() == path
    }
}

fn protection_cached(settings: &Settings, cache: &mut ProtectionCache) -> Protection {
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
        let len = GetClassNameW(hwnd, &mut class).max(0) as usize;
        let class_slice = &class[..len];
        if [
            "WorkerW",
            "Progman",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "XamlExplorerHostIslandWindow",
            "MultitaskingViewFrame",
            "TaskSwitcherWnd",
        ]
        .iter()
        .any(|name| eq_wide_ascii(class_slice, name))
        {
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
                cache.path.as_deref()
            } else {
                let queried = query_image_path(pid);
                *cache = ProtectionCache {
                    hwnd: hwnd.0 as usize,
                    pid,
                    path: queried,
                };
                cache.path.as_deref()
            };
            if path.is_some_and(|path| {
                settings
                    .protected_apps
                    .iter()
                    .any(|rule| rule_matches_path(rule, path))
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
        let mut stack = [0u16; 512];
        let mut length = stack.len() as u32;
        match QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(stack.as_mut_ptr()),
            &mut length,
        ) {
            Ok(()) => Some(
                String::from_utf16_lossy(&stack[..length as usize])
                    .replace('/', "\\")
                    .to_lowercase(),
            ),
            Err(err) if err.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {
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
            Err(_) => None,
        }
    }
}

pub(super) fn fullscreen_rect(rect: RECT, screen: RECT, tolerance: i32) -> bool {
    (rect.left - screen.left).abs() <= tolerance
        && (rect.top - screen.top).abs() <= tolerance
        && (rect.right - screen.right).abs() <= tolerance
        && (rect.bottom - screen.bottom).abs() <= tolerance
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
                notify(NativeSignal::ForegroundChanged);
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
                notify(if message == WM_DISPLAYCHANGE {
                    NativeSignal::DisplayChanged
                } else {
                    NativeSignal::SystemThemeChanged
                });
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
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                let _ = DispatchMessageW(&message);
            }
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
