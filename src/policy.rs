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
    fn signals(strict: bool, geometry: bool, notification: Notification) -> Signals {
        Signals {
            strict_rule: strict,
            fullscreen_geometry: geometry,
            notification,
            ..Default::default()
        }
    }
    #[test]
    fn auxiliary_sessions_block_all_automatic_hides_but_not_strict_protection() {
        for guard in [
            AutoHideGuard {
                editor: true,
                ..Default::default()
            },
            AutoHideGuard {
                settings: true,
                ..Default::default()
            },
            AutoHideGuard {
                name: true,
                ..Default::default()
            },
            AutoHideGuard {
                file_dialog: true,
                ..Default::default()
            },
        ] {
            assert!(guard.paused());
            let mut open = signals(false, true, Notification::Other);
            open.auxiliary_open = guard.auxiliary_open();
            assert_eq!(decide(open), Protection::Normal);
            let mut strict = signals(true, true, Notification::RunningD3dFullScreen);
            strict.auxiliary_open = guard.auxiliary_open();
            assert_eq!(
                decide(strict),
                Protection::Strict,
                "an configured game rule outranks an open editor"
            );
        }
        assert_eq!(
            decide(signals(false, true, Notification::Other)),
            Protection::Fullscreen
        );
        assert!(!AutoHideGuard::default().paused());
    }
    fn rank(protection: Protection) -> u8 {
        match protection {
            Protection::Normal => 0,
            Protection::Fullscreen => 1,
            Protection::Strict => 2,
        }
    }
    #[test]
    fn notification_can_only_escalate_and_never_grants_visibility() {
        for strict in [false, true] {
            for geometry in [false, true] {
                for auxiliary in [false, true] {
                    let base = Signals {
                        strict_rule: strict,
                        fullscreen_geometry: geometry,
                        notification: Notification::Unavailable,
                        auxiliary_open: auxiliary,
                        bypass: false,
                    };
                    for notification in [
                        Notification::Unavailable,
                        Notification::Other,
                        Notification::Busy,
                        Notification::RunningD3dFullScreen,
                        Notification::PresentationMode,
                    ] {
                        let mut signals = base;
                        signals.notification = notification;
                        assert!(
                            rank(decide(signals)) >= rank(decide(base)),
                            "a notification value weakened protection: {signals:?}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn fullscreen_video_and_presentation_states_protect_without_geometry() {
        for notification in [
            Notification::RunningD3dFullScreen,
            Notification::PresentationMode,
        ] {
            assert_eq!(
                decide(signals(false, false, notification)),
                Protection::Fullscreen,
                "a lower-resolution exclusive mode misses the monitor rectangle"
            );
        }
        // The master switch is enforced in the platform layer by forcing both signals off.
        assert_eq!(
            decide(signals(false, false, Notification::Unavailable)),
            Protection::Normal
        );
    }
    #[test]
    fn busy_only_holds_the_release_and_a_query_failure_keeps_other_signals() {
        assert!(signals(false, false, Notification::Busy).hold_release());
        assert!(!signals(false, false, Notification::Unavailable).hold_release());
        assert!(!signals(false, false, Notification::RunningD3dFullScreen).hold_release());
        // Row 7: the query failed, so geometry and rules must still be able to protect.
        assert_eq!(
            decide(signals(true, false, Notification::Unavailable)),
            Protection::Strict
        );
        assert_eq!(
            decide(signals(false, true, Notification::Unavailable)),
            Protection::Fullscreen
        );
    }
    #[test]
    fn manual_bypass_outranks_everything_for_one_run_only() {
        let mut all = Signals {
            strict_rule: true,
            fullscreen_geometry: true,
            notification: Notification::RunningD3dFullScreen,
            bypass: true,
            ..Default::default()
        };
        assert_eq!(decide(all), Protection::Normal);
        all.bypass = false;
        assert_eq!(decide(all), Protection::Strict);
        let mut p = DisplayPolicy::default();
        assert!(p.set_bypass(true));
        assert!(!p.set_bypass(true), "repeating the request changes nothing");
        assert!(p.set_bypass(false));
    }
    #[test]
    fn repeated_external_focus_does_not_hide_auxiliary_session_and_close_starts_fresh_delay() {
        use crate::docking::FocusHide;
        for hide_on_loss in [false, true] {
            for field in 0..4 {
                let guard = AutoHideGuard {
                    editor: field == 0,
                    settings: field == 1,
                    name: field == 2,
                    file_dialog: field == 3,
                    ..Default::default()
                };
                let mut policy = DisplayPolicy::default();
                let mut fade = FocusHide::default();
                let start = Instant::now();
                // Include a fade already in flight when the child is opened.
                fade.opacity(hide_on_loss, false, true, start);
                let fading = fade.opacity(
                    hide_on_loss,
                    false,
                    true,
                    start + Duration::from_millis(150),
                );
                assert!((fading - if hide_on_loss { 0.5 } else { 1. }).abs() < 0.001);
                for tick in 1..=100 {
                    let now = start + Duration::from_millis(150 + tick * 50);
                    let observed = Signals {
                        fullscreen_geometry: tick % 2 == 0,
                        auxiliary_open: guard.auxiliary_open(),
                        ..Default::default()
                    };
                    let changed = policy.update(decide(observed), now, observed.hold_release());
                    assert!(!changed);
                    assert_eq!(policy.protection(), Protection::Normal);
                    assert_eq!(
                        fade.opacity(hide_on_loss && !guard.paused(), false, true, now),
                        1.
                    );
                    assert!(
                        guard.paused(),
                        "delayed launch completion must also stay visible"
                    );
                }
                let close = start + Duration::from_secs(6);
                assert_eq!(
                    fade.opacity(hide_on_loss, false, true, close),
                    1.,
                    "no stale fade when the last child closes"
                );
                assert_eq!(
                    fade.opacity(
                        hide_on_loss,
                        false,
                        true,
                        close + Duration::from_millis(250)
                    ),
                    if hide_on_loss { 0. } else { 1. }
                );
                assert!(policy.update(
                    decide(Signals {
                        strict_rule: true,
                        auxiliary_open: guard.auxiliary_open(),
                        ..Default::default()
                    }),
                    close,
                    false
                ));
                assert!(!policy.hotkey_enabled());
            }
        }
    }
    #[test]
    fn every_entry_point_is_blocked_and_old_requests_expire() {
        let mut p = DisplayPolicy::default();
        let old = p.ticket();
        let now = Instant::now();
        p.update(Protection::Strict, now, false);
        for source in [
            ShowSource::Startup,
            ShowSource::Hotkey,
            ShowSource::Tray,
            ShowSource::SecondaryInstance,
            ShowSource::Editor,
            ShowSource::Settings,
            ShowSource::Restore,
            ShowSource::NativeMenu,
            ShowSource::Relocate,
            ShowSource::Drop,
        ] {
            assert!(!p.allows(source, p.ticket()));
        }
        assert!(!p.hotkey_enabled());
        p.update(Protection::Normal, now, false);
        p.update(Protection::Normal, now + Duration::from_secs(1), false);
        assert!(p.hotkey_enabled());
        assert!(!p.allows(ShowSource::Hotkey, old));
        assert!(p.allows(ShowSource::Tray, p.ticket()));
    }
    #[test]
    fn transient_loss_does_not_release_or_hold_forever() {
        let mut p = DisplayPolicy::default();
        let now = Instant::now();
        p.update(Protection::Fullscreen, now, false);
        p.update(Protection::Normal, now, false);
        p.update(Protection::Normal, now + Duration::from_millis(300), false);
        assert_eq!(p.protection(), Protection::Fullscreen);
        p.update(Protection::Normal, now + Duration::from_secs(2), false);
        assert_eq!(p.protection(), Protection::Normal);
    }
    #[test]
    fn transient_loss_uses_the_longer_hold_deadline() {
        let mut p = DisplayPolicy::default();
        let now = Instant::now();
        p.update(Protection::Fullscreen, now, true);
        // Past the ordinary 900 ms window, still inside the held 3000 ms one.
        p.update(Protection::Normal, now + Duration::from_millis(1500), true);
        assert_eq!(
            p.protection(),
            Protection::Fullscreen,
            "a sticky busy flag keeps protection a little longer before releasing"
        );
        p.update(Protection::Normal, now + Duration::from_millis(3100), true);
        assert_eq!(
            p.protection(),
            Protection::Fullscreen,
            "the hold counts from the first weaker observation, not from this call"
        );
        p.update(Protection::Normal, now + Duration::from_millis(4600), true);
        assert_eq!(
            p.protection(),
            Protection::Normal,
            "but the hold is finite: a live game process alone cannot hold it open"
        );
    }
    #[test]
    fn strict_protection_survives_a_brief_fullscreen_overlay() {
        let mut p = DisplayPolicy::default();
        let now = Instant::now();
        p.update(Protection::Strict, now, false);
        p.update(
            Protection::Fullscreen,
            now + Duration::from_millis(200),
            false,
        );
        assert!(!p.hotkey_enabled());
        p.update(Protection::Fullscreen, now + Duration::from_secs(2), false);
        assert_eq!(p.protection(), Protection::Fullscreen);
    }
}
