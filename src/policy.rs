//! All UI display requests are admitted here, including delayed/native callbacks.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protection {
    Normal,
    Fullscreen,
    Strict,
}

/// `SHQueryUserNotificationState`, as an auxiliary signal only. `Unavailable` covers a failed
/// query, which must not disable the rules that come from geometry or an application match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Notification {
    #[default]
    Unavailable,
    /// Set by media players and presentations too, and can outlive them.
    Busy,
    RunningD3dFullScreen,
    PresentationMode,
    Other,
}

/// Raw observations for one evaluation. The platform layer fills `strict_rule`,
/// `fullscreen_geometry` and `notification`; the application fills the session flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    pub strict_rule: bool,
    pub fullscreen_geometry: bool,
    pub notification: Notification,
    pub auxiliary_open: bool,
    pub bypass: bool,
}
impl Signals {
    /// `QUNS_BUSY` is set by media players and presentations as well as games and can outlive
    /// them, so on its own it only justifies a longer wait before releasing protection.
    pub fn hold_release(&self) -> bool {
        self.notification == Notification::Busy
    }
}

/// Combined verdict, first matching row wins. There is deliberately no row where a
/// notification value produces `Normal`: being allowed to notify is not being allowed to take
/// the foreground.
pub fn decide(signals: Signals) -> Protection {
    if signals.bypass {
        Protection::Normal
    } else if signals.strict_rule {
        Protection::Strict
    } else if signals.auxiliary_open {
        Protection::Normal
    } else if signals.fullscreen_geometry
        || matches!(
            signals.notification,
            Notification::RunningD3dFullScreen | Notification::PresentationMode
        )
    {
        Protection::Fullscreen
    } else {
        Protection::Normal
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AutoHideGuard {
    pub editor: bool,
    pub settings: bool,
    pub name: bool,
    pub file_dialog: bool,
    pub pinned: bool,
    pub interaction: bool,
}
impl AutoHideGuard {
    pub fn auxiliary_open(self) -> bool {
        self.editor || self.settings || self.name || self.file_dialog
    }
    pub fn paused(self) -> bool {
        self.auxiliary_open() || self.pinned || self.interaction
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowSource {
    Startup,
    Hotkey,
    Tray,
    SecondaryInstance,
    Editor,
    Settings,
    Restore,
    /// Appended so the integer the metrics log records stays comparable across releases.
    NativeMenu,
    Relocate,
    Drop,
}

pub struct DisplayPolicy {
    protection: Protection,
    release_after: Option<Instant>,
    generation: u64,
    bypass: bool,
}
impl Default for DisplayPolicy {
    fn default() -> Self {
        Self {
            protection: Protection::Normal,
            release_after: None,
            generation: 0,
            bypass: false,
        }
    }
}
impl DisplayPolicy {
    pub fn update(&mut self, signal: Protection, now: Instant, hold: bool) -> bool {
        let previous = self.protection;
        let strengthening = signal == Protection::Strict
            || (signal == Protection::Fullscreen && self.protection == Protection::Normal);
        if signal == self.protection || strengthening {
            self.release_after = None;
            self.protection = signal;
        } else {
            let wait = if hold { 3000 } else { 900 };
            let deadline = self
                .release_after
                .get_or_insert(now + Duration::from_millis(wait));
            if now >= *deadline {
                self.protection = signal;
                self.release_after = None;
            }
        }
        if previous != self.protection {
            self.generation += 1;
        }
        previous != self.protection
    }
    pub fn protection(&self) -> Protection {
        self.protection
    }
    pub fn ticket(&self) -> u64 {
        self.generation
    }
    pub fn allows(&self, _source: ShowSource, ticket: u64) -> bool {
        self.protection == Protection::Normal && ticket == self.generation
    }
    pub fn hotkey_enabled(&self) -> bool {
        self.protection != Protection::Strict
    }
    /// Manual override for one run only: evaluation keeps running, the verdict is pinned to
    /// `Normal`. Never persisted, so a launcher cannot be left invisible over a game by accident.
    pub fn set_bypass(&mut self, bypass: bool) -> bool {
        if self.bypass == bypass {
            return false;
        }
        self.bypass = bypass;
        self.release_after = None;
        self.generation += 1;
        true
    }
    pub fn bypass(&self) -> bool {
        self.bypass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_prioritizes_bypass_strict_and_fullscreen() {
        assert_eq!(
            decide(Signals {
                strict_rule: true,
                fullscreen_geometry: true,
                notification: Notification::RunningD3dFullScreen,
                bypass: true,
                ..Default::default()
            }),
            Protection::Normal
        );
        assert_eq!(
            decide(Signals {
                strict_rule: true,
                auxiliary_open: true,
                ..Default::default()
            }),
            Protection::Strict
        );
        assert_eq!(
            decide(Signals {
                fullscreen_geometry: true,
                auxiliary_open: true,
                ..Default::default()
            }),
            Protection::Normal
        );
        assert_eq!(
            decide(Signals {
                fullscreen_geometry: true,
                ..Default::default()
            }),
            Protection::Fullscreen
        );
    }

    #[test]
    fn display_policy_debounces_release_and_blocks_sources() {
        let mut p = DisplayPolicy::default();
        let old = p.ticket();
        let now = Instant::now();
        p.update(Protection::Strict, now, false);
        assert!(!p.hotkey_enabled());
        assert!(!p.allows(ShowSource::Hotkey, p.ticket()));

        p.update(Protection::Normal, now, false);
        assert_eq!(p.protection(), Protection::Strict);
        p.update(Protection::Normal, now + Duration::from_secs(2), false);
        assert_eq!(p.protection(), Protection::Normal);
        assert!(!p.allows(ShowSource::Hotkey, old));
        assert!(p.allows(ShowSource::Hotkey, p.ticket()));
    }
}
