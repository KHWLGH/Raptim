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
                if hidden { 70. } else { 60. }
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
        if t == 0. && (self.from - self.to).abs() >= 0.0003 {
            return (self.from, self.velocity, false);
        }
        let displacement = self.from - self.to;
        let b = self.velocity + self.omega * displacement;
        let decay = (-self.omega * t).exp();
        let position = self.to + (displacement + b * t) * decay;
        let velocity = (self.velocity - self.omega * b * t) * decay;
        if (position - self.to).abs() < 0.0003 && velocity.abs() < 0.02 {
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
    #[test]
    fn composited_slide_only_clips_visible_spill() {
        let monitor = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let work = Rect {
            height: 1040,
            ..monitor
        };
        let slide = Rect {
            x: -200,
            y: 100,
            width: 400,
            height: 600,
        };
        assert!(!needs_work_clip(slide, work, [monitor]));
        // An adjacent display requires a clip; a gap or vertically offset display does not.
        assert!(needs_work_clip(
            slide,
            work,
            [
                monitor,
                Rect {
                    x: -1920,
                    ..monitor
                }
            ]
        ));
        assert!(!needs_work_clip(
            slide,
            work,
            [
                monitor,
                Rect {
                    x: -2120,
                    ..monitor
                }
            ]
        ));
        assert!(!needs_work_clip(
            slide,
            work,
            [
                monitor,
                Rect {
                    x: -1920,
                    y: 800,
                    ..monitor
                }
            ]
        ));
        assert!(needs_work_clip(Rect { y: 800, ..slide }, work, [monitor]));
        assert!(!needs_work_clip(Rect { x: 1920, ..slide }, work, [monitor]));
        assert!(!needs_work_clip(Rect { x: 0, ..slide }, work, [monitor]));
        assert!(!needs_work_clip(
            Rect {
                x: 100,
                y: -300,
                ..slide
            },
            work,
            [monitor]
        ));
        assert!(needs_work_clip(
            Rect {
                x: 100,
                y: -300,
                ..slide
            },
            work,
            [
                monitor,
                Rect {
                    y: -1080,
                    ..monitor
                }
            ]
        ));
    }
    #[test]
    fn disabling_focus_hide_or_regaining_focus_cancels_pending_fade() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut state = FocusHide::default();
        assert_eq!(state.opacity(true, false, true, now), 1.);
        assert!(state.opacity(true, false, true, now + Duration::from_millis(150)) < 1.);
        assert_eq!(
            state.opacity(false, false, true, now + Duration::from_secs(1)),
            1.
        );
        assert_eq!(
            state.opacity(true, false, true, now + Duration::from_secs(2)),
            1.
        );
        assert_eq!(
            state.opacity(true, true, true, now + Duration::from_secs(3)),
            1.
        );
        assert_eq!(
            state.opacity(true, false, false, now + Duration::from_secs(4)),
            1.
        );
        assert_eq!(
            state.opacity(true, false, false, now + Duration::from_millis(4100)),
            0.
        );
        for n in 5..100 {
            assert_eq!(
                state.opacity(false, false, true, now + Duration::from_secs(n)),
                1.
            );
        }
    }
    #[test]
    fn invisible_reveal_is_local_and_width_is_physical() {
        let rect = Rect {
            x: -1920,
            y: 100,
            width: 800,
            height: 1200,
        };
        for edge in [Edge::Left, Edge::Right, Edge::Top] {
            for width in [0, 1, 6, 12] {
                let band = reveal_band(edge, rect, width);
                assert_eq!(
                    if edge == Edge::Top {
                        band.height
                    } else {
                        band.width
                    },
                    width.max(1)
                );
                assert!(!contains(band, (-1920, 99)));
            }
        }
    }
    #[test]
    fn approach_band_spans_the_work_area_not_the_window() {
        let work = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let docked = Rect {
            x: -1920,
            y: 50,
            width: 440,
            height: 640,
        };
        // Vertically outside the window but still on the edge: fine rate has to come back.
        assert!(!contains(reveal_band(Edge::Left, docked, 6), (-1919, 900)));
        assert!(contains(
            approach_band(Edge::Left, work, 6, 96),
            (-1919, 900)
        ));
        assert!(!contains(
            approach_band(Edge::Left, work, 6, 96),
            (-1818, 900)
        ));
        assert!(contains(approach_band(Edge::Right, work, 6, 96), (-6, 900)));
        assert!(!contains(
            approach_band(Edge::Right, work, 6, 96),
            (-103, 900)
        ));
        assert!(contains(approach_band(Edge::Top, work, 0, 96), (-1000, 95)));
        assert!(!contains(
            approach_band(Edge::Top, work, 0, 96),
            (-1000, 97)
        ));
        assert_eq!(approach_band(Edge::Top, work, 0, 0).height, 1);
    }
    #[test]
    fn motion_can_reverse_without_jumping() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let out = Motion::new(0., 0., true, now, true);
        let half = now + Duration::from_millis(25);
        let (value, velocity, done) = out.sample(half);
        assert!(!done);
        let reverse = Motion::new(value, velocity, false, half, true);
        assert_eq!(reverse.value(half).0, value);
        assert!((reverse.sample(half).1 - velocity).abs() < 0.0001);
        assert_eq!(reverse.value(half + Duration::from_millis(400)), (0., true));
    }
    #[test]
    fn motion_starts_softly_settles_and_never_leaves_travel_bounds() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        for hidden in [false, true] {
            let start = if hidden { 0. } else { 1. };
            let motion = Motion::new(start, 0., hidden, now, true);
            assert_eq!(motion.sample(now), (start, 0., false));
            assert!((motion.value(now + Duration::from_millis(8)).0 - start).abs() < 0.12);
            let mut previous = start;
            for ms in 1..400 {
                let (position, _, _) = motion.sample(now + Duration::from_millis(ms));
                assert!((0. ..=1.).contains(&position));
                assert!(if hidden {
                    position >= previous
                } else {
                    position <= previous
                });
                previous = position;
            }
            assert!(motion.sample(now + Duration::from_millis(200)).2);
            assert_eq!(
                Motion::new(start, 0., hidden, now, false).value(now),
                (1. - start, true)
            );
        }
        let mut motion = Motion::new(0., 0., true, now, true);
        for ms in (7..300).step_by(7) {
            let time = now + Duration::from_millis(ms);
            let (position, velocity, _) = motion.sample(time);
            assert!((0. ..=1.).contains(&position));
            motion = Motion::new(position, velocity, motion.to == 0., time, true);
            assert_eq!(motion.sample(time).0, position);
        }
    }
    #[test]
    fn negative_monitor_and_reveal_strip() {
        let work = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let r = Rect {
            x: -1918,
            y: 50,
            width: 400,
            height: 640,
        };
        let (edge, r) = dock(r, work, 12).unwrap();
        assert_eq!(hidden_position(edge, r, 3), (-2317, 50));
        assert!(contains(r, (-1920, 80)));
        assert!(dock(Rect { x: -1000, ..r }, work, 12).is_none());
    }
}
