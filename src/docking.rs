use crate::windowing::Rect;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Left,
    Right,
    Top,
}
pub fn dock(rect: Rect, work: Rect, threshold: i32) -> Option<(Edge, Rect)> {
    let mut r = rect;
    let edge = if r.x <= work.x + threshold {
        r.x = work.x;
        Edge::Left
    } else if r.x + r.width >= work.x + work.width - threshold {
        r.x = work.x + work.width - r.width;
        Edge::Right
    } else if r.y <= work.y + threshold {
        r.y = work.y;
        Edge::Top
    } else {
        return None;
    };
    r.y = r.y.clamp(work.y, work.y + (work.height - r.height).max(0));
    r.x = r.x.clamp(work.x, work.x + (work.width - r.width).max(0));
    Some((edge, r))
}
pub const DOCK_OVERHANG_DIP: f32 = 10.0;

pub fn overhang_px(scale: f32) -> i32 {
    (DOCK_OVERHANG_DIP * scale.max(1.0)).round() as i32
}

/// Extends the native window past the docked screen edge so its compositor-rounded
/// corners on that edge stay off-screen throughout both rest and sliding.
pub fn docked_hwnd_rect(edge: Edge, visible: Rect, overhang: i32) -> Rect {
    let pad = overhang.max(0);
    match edge {
        Edge::Left => Rect {
            x: visible.x - pad,
            y: visible.y,
            width: visible.width + pad,
            height: visible.height,
        },
        Edge::Right => Rect {
            x: visible.x,
            y: visible.y,
            width: visible.width + pad,
            height: visible.height,
        },
        Edge::Top => Rect {
            x: visible.x,
            y: visible.y - pad,
            width: visible.width,
            height: visible.height + pad,
        },
    }
}

pub fn visible_rect_from_hwnd(edge: Edge, hwnd: Rect, overhang: i32) -> Rect {
    let pad = overhang.max(0);
    match edge {
        Edge::Left => Rect {
            x: hwnd.x + pad,
            y: hwnd.y,
            width: (hwnd.width - pad).max(1),
            height: hwnd.height.max(1),
        },
        Edge::Right => Rect {
            x: hwnd.x,
            y: hwnd.y,
            width: (hwnd.width - pad).max(1),
            height: hwnd.height.max(1),
        },
        Edge::Top => Rect {
            x: hwnd.x,
            y: hwnd.y + pad,
            width: hwnd.width.max(1),
            height: (hwnd.height - pad).max(1),
        },
    }
}

pub fn hidden_hwnd_position(edge: Edge, visible: Rect, strip: i32, overhang: i32) -> (i32, i32) {
    let hwnd = docked_hwnd_rect(edge, visible, overhang);
    match edge {
        Edge::Left => (visible.x - hwnd.width + strip, hwnd.y),
        Edge::Right => (visible.x + visible.width - strip, hwnd.y),
        Edge::Top => (hwnd.x, visible.y - hwnd.height + strip),
    }
}

pub fn hidden_position(edge: Edge, rect: Rect, strip: i32) -> (i32, i32) {
    match edge {
        Edge::Left => (rect.x - rect.width + strip, rect.y),
        Edge::Right => (rect.x + rect.width - strip, rect.y),
        Edge::Top => (rect.x, rect.y - rect.height + strip),
    }
}
pub fn contains(r: Rect, p: (i32, i32)) -> bool {
    p.0 >= r.x && p.0 < r.x + r.width && p.1 >= r.y && p.1 < r.y + r.height
}

/// Clip only pixels that would actually be visible outside the chosen work area.
/// Off-desktop pixels need no HRGN, so DWM can keep anti-aliased corners while sliding.
pub fn needs_work_clip(window: Rect, work: Rect, monitors: impl IntoIterator<Item = Rect>) -> bool {
    monitors.into_iter().any(|monitor| {
        let left = window.x.max(monitor.x);
        let top = window.y.max(monitor.y);
        let right = (window.x + window.width).min(monitor.x + monitor.width);
        let bottom = (window.y + window.height).min(monitor.y + monitor.height);
        left < right
            && top < bottom
            && (left < work.x
                || top < work.y
                || right > work.x + work.width
                || bottom > work.y + work.height)
    })
}

pub fn reveal_band(edge: Edge, rect: Rect, width: i32) -> Rect {
    let width = width.max(1);
    match edge {
        Edge::Left => Rect { width, ..rect },
        Edge::Right => Rect {
            x: rect.x + rect.width - width,
            width,
            ..rect
        },
        Edge::Top => Rect {
            height: width,
            ..rect
        },
    }
}

/// The reveal band grown by `margin` and stretched across the whole work area. A collapsed
/// launcher polls at half rate while the pointer is outside this, so it has to span the edge and
/// not just the window: someone sliding along the screen edge from outside the window's reach
/// must be back on the fine rate before the reveal dwell can elapse.
pub fn approach_band(edge: Edge, work: Rect, width: i32, margin: i32) -> Rect {
    let width = (width + margin).max(1);
    match edge {
        Edge::Left => Rect { width, ..work },
        Edge::Right => Rect {
            x: work.x + work.width - width,
            width,
            ..work
        },
        Edge::Top => Rect {
            height: width,
            ..work
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub from: f32,
    pub to: f32,
    pub started: std::time::Instant,
    pub velocity: f32,
    omega: f32,
}

/// A focus transition cannot outlive the setting or an application interaction.
#[derive(Default)]
pub struct FocusHide {
    lost_at: Option<std::time::Instant>,
}
impl FocusHide {
    pub fn reset(&mut self) {
        self.lost_at = None;
    }
    pub fn opacity(
        &mut self,
        enabled: bool,
        focused: bool,
        animations: bool,
        now: std::time::Instant,
    ) -> f32 {
        if !enabled || focused {
            self.reset();
            return 1.;
        }
        let start = *self.lost_at.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start).as_secs_f32();
        if elapsed < 0.1 {
            1.
        } else if animations {
            (1. - (elapsed - 0.1) / 0.1).clamp(0., 1.)
        } else {
            0.
        }
    }
}

/// Keeps an explicitly summoned (hotkey / tray / secondary instance / startup)
/// docked window expanded while the pointer starts outside it, instead of
/// immediately auto-collapsing after `edge_collapse_delay_ms`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeHold {
    #[default]
    Idle,
    /// Summoned explicitly; waiting for focus confirmation or a post-settle grace timeout.
    Pending(std::time::Instant),
    /// Focus was confirmed after summon; holds until focus is lost or the pointer enters.
    Focused,
}

impl EdgeHold {
    /// Post-settle grace window when OS foreground activation is delayed or blocked (e.g. UIPI).
    pub const PENDING_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

    pub fn arm(&mut self, now: std::time::Instant, focused: bool) {
        *self = if focused {
            Self::Focused
        } else {
            Self::Pending(now)
        };
    }

    pub fn clear(&mut self) {
        *self = Self::Idle;
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Keeps a pending grace timer from aging out while auto-hide is paused.
    pub fn refresh_pending(&mut self, now: std::time::Instant) {
        if matches!(self, Self::Pending(_)) {
            *self = Self::Pending(now);
        }
    }

    /// Advances the hold state for one expanded docked tick.
    /// Returns `true` when auto-collapse should be suppressed for this tick.
    pub fn hold(
        &mut self,
        pointer_inside: bool,
        moving: bool,
        focused: bool,
        now: std::time::Instant,
    ) -> bool {
        match *self {
            Self::Idle => false,
            Self::Pending(since) => {
                if focused {
                    if !moving && pointer_inside {
                        *self = Self::Idle;
                        false
                    } else {
                        *self = Self::Focused;
                        true
                    }
                } else if moving {
                    *self = Self::Pending(now);
                    true
                } else if pointer_inside
                    || now.saturating_duration_since(since) >= Self::PENDING_GRACE
                {
                    *self = Self::Idle;
                    false
                } else {
                    true
                }
            }
            Self::Focused => {
                if !focused || (!moving && pointer_inside) {
                    *self = Self::Idle;
                    false
                } else {
                    true
                }
            }
        }
    }
}

/// Visual frame state for a docked window so resting collapsed strips never cast
/// a DWM drop shadow onto the desktop edge, while keeping rounded corners intact
/// throughout both sliding and expanded rest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollapsedAppearance {
    /// Expanded, sliding, or undocked: DWM rounded corners and shadow enabled, uncloaked.
    #[default]
    Normal,
    /// Collapsed at rest with a visible edge strip (`strip > 0`): uncloaked, with DWM
    /// non-client rendering/shadow disabled so only the thin strip appears.
    StripOnly,
    /// Collapsed at rest with zero visible strip (`strip == 0`): cloaked in DWM so
    /// neither window pixels nor shadow appear on the screen edge.
    Cloaked,
}

pub fn collapsed_appearance(
    docked: bool,
    edge_hidden: bool,
    progress: f32,
    moving: bool,
    strip: i32,
) -> CollapsedAppearance {
    if !docked {
        return CollapsedAppearance::Normal;
    }
    if strip <= 0 {
        return if edge_hidden && !moving {
            CollapsedAppearance::Cloaked
        } else {
            CollapsedAppearance::Normal
        };
    }
    // StripOnly drops DWM rounded corners and drop shadow in the tail band of motion so
    // resting strip appearance engages while gliding rather than popping once settled.
    let strip_only = if moving {
        if edge_hidden {
            progress >= 0.82
        } else {
            progress > 0.85
        }
    } else {
        edge_hidden
    };
    if strip_only {
        CollapsedAppearance::StripOnly
    } else {
        CollapsedAppearance::Normal
    }
}

impl Motion {
    pub fn new(
        from: f32,
        velocity: f32,
        hidden: bool,
        now: std::time::Instant,
        animated: bool,
    ) -> Self {
        Self {
            from,
            velocity,
            to: if hidden { 1. } else { 0. },
            started: now,
            omega: if animated {
                if hidden { 44. } else { 48. }
            } else {
                0.
            },
        }
    }
    /// Analytic critically damped motion: independent of frame rate and carrying
    /// velocity through a reversal. No bounce, initial speed jump or linear tail.
    pub fn sample(self, now: std::time::Instant) -> (f32, f32, bool) {
        let t = now.saturating_duration_since(self.started).as_secs_f32();
        if self.omega == 0. || t >= 0.4 {
            return (self.to, 0., true);
        }
        if t == 0. && (self.from - self.to).abs() >= 0.0018 {
            return (self.from, self.velocity, false);
        }
        let displacement = self.from - self.to;
        let b = self.velocity + self.omega * displacement;
        let decay = (-self.omega * t).exp();
        let position = self.to + (displacement + b * t) * decay;
        let velocity = (self.velocity - self.omega * b * t) * decay;
        if (position - self.to).abs() < 0.0018 && velocity.abs() < 0.06 {
            return (self.to, 0., true);
        }
        // A reversal near a physical edge may still carry outward momentum.
        // Keep the real window inside its travel limits without overshoot.
        (
            position.clamp(0., 1.),
            if !(0. ..=1.).contains(&position) {
                0.
            } else {
                velocity
            },
            false,
        )
    }
    pub fn value(self, now: std::time::Instant) -> (f32, bool) {
        let (position, _, done) = self.sample(now);
        (position, done)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn dock_and_overhang_geometry() {
        let work = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let rect = Rect {
            x: 4,
            y: 100,
            width: 440,
            height: 640,
        };
        let (edge, docked) = dock(rect, work, 12).unwrap();
        assert_eq!(edge, Edge::Left);
        assert_eq!(docked.x, 0);

        let pad = overhang_px(1.5);
        let hwnd = docked_hwnd_rect(edge, docked, pad);
        assert_eq!(visible_rect_from_hwnd(edge, hwnd, pad), docked);
        let hidden = hidden_hwnd_position(edge, docked, 6, pad);
        assert_eq!(hidden.0 + hwnd.width - docked.x, 6);
        assert_eq!(reveal_band(edge, docked, 6).width, 6);
    }

    #[test]
    fn motion_and_focus_hide_settle() {
        let now = Instant::now();
        let motion = Motion::new(0., 0., true, now, true);
        let (mid, vel, done) = motion.sample(now + Duration::from_millis(25));
        assert!(!done && (0. ..=1.).contains(&mid));
        let rev = Motion::new(mid, vel, false, now + Duration::from_millis(25), true);
        assert_eq!(rev.value(now + Duration::from_millis(400)), (0., true));

        let mut fade = FocusHide::default();
        assert_eq!(fade.opacity(true, false, true, now), 1.);
        assert_eq!(
            fade.opacity(true, false, true, now + Duration::from_millis(300)),
            0.
        );
        assert_eq!(
            fade.opacity(true, true, true, now + Duration::from_secs(1)),
            1.
        );
    }
}
