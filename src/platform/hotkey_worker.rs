//! Dedicated Win32 message-pump thread for global hotkey registration and conflict fallback.
use super::*;

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
    /// an error or never arrives, so the worst it can do is cause one redundant re-send.
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
                                Err(_) => Err(crate::i18n::t(
                                    "此呼出热键已被占用或不可用，已保留原热键",
                                    "Hotkey already in use or unavailable; kept previous shortcut.",
                                )
                                .into()),
                            }
                        };
                        let _ = request.reply.send(result);
                    }
                } else if message.message == WM_HOTKEY
                    && registered
                        .as_ref()
                        .is_some_and(|(id, _)| *id as usize == message.wParam.0)
                {
                    worker_notify(NativeSignal::HotkeyPressed);
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
            notify(NativeSignal::StatusMessage(format!(
                "{error}{}",
                crate::i18n::t("；可用托盘找回窗口", " (use tray icon to open window)")
            )));
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
            Ok(Err(text)) => {
                self.requested = None;
                Err(text)
            }
            Err(e) => {
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
