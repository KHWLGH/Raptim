//! Keep ordinary clicks and menu dismissal from turning into a reorder gesture, and keep
//! input-method composition from reaching the launcher's own key handling.
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct DragGuard {
    press: Option<((f64, f64), Instant)>,
    armed: bool,
    moved: bool,
}
impl DragGuard {
    pub fn press(&mut self, point: (f64, f64), now: Instant) {
        self.reset();
        self.press = Some((point, now));
    }
    pub fn suppress_move(&mut self, point: (f64, f64), now: Instant) -> bool {
        let Some((start, time)) = self.press else {
            return false;
        };
        let distance = (point.0 - start.0).hypot(point.1 - start.1);
        self.moved |= distance >= 12.;
        self.armed |= distance >= 12. && now.duration_since(time) >= Duration::from_millis(180);
        !self.armed
    }
    /// A quick movement beyond the click area is neither a launch nor a reorder.
    pub fn release(&mut self) -> bool {
        let cancel_click = self.moved && !self.armed;
        self.reset();
        cancel_click
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
/// winit delivers an empty preedit string both while a composition is being replaced and after
/// it has ended; only the presence of the cursor area tells the two apart.
pub enum ImeSignal {
    Preedit { has_cursor: bool },
    Commit,
    Disabled,
}
#[derive(Default)]
pub struct CompositionGuard {
    active: bool,
}
impl CompositionGuard {
    pub fn apply(&mut self, signal: ImeSignal) -> bool {
        self.active = match signal {
            ImeSignal::Preedit { has_cursor } => has_cursor,
            ImeSignal::Commit | ImeSignal::Disabled => false,
        };
        self.active
    }
    pub fn active(&self) -> bool {
        self.active
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jitter_quick_motion_deliberate_drag_and_cancellation() {
        let now = Instant::now();
        let mut guard = DragGuard::default();
        guard.press((0., 0.), now);
        assert!(guard.suppress_move((9., 0.), now + Duration::from_millis(500)));
        assert!(!guard.release());
        guard.press((0., 0.), now);
        assert!(guard.suppress_move((90., 0.), now + Duration::from_millis(50)));
        assert!(guard.release());
        guard.press((0., 0.), now);
        assert!(!guard.suppress_move((15., 0.), now + Duration::from_millis(200)));
        assert!(!guard.release());
        guard.press((0., 0.), now);
        guard.reset();
        assert!(!guard.suppress_move((90., 0.), now));
        assert!(!guard.release());
    }
    #[test]
    fn empty_preedit_with_a_cursor_area_is_still_composing() {
        let mut guard = CompositionGuard::default();
        assert!(!guard.active());
        assert!(guard.apply(ImeSignal::Preedit { has_cursor: true }));
        // The regression: an empty replacement string used to end composition early, so the
        // Enter that confirms a candidate also launched the selected item.
        assert!(guard.apply(ImeSignal::Preedit { has_cursor: true }));
        assert!(guard.active());
        assert!(!guard.apply(ImeSignal::Preedit { has_cursor: false }));
        guard.apply(ImeSignal::Preedit { has_cursor: true });
        assert!(!guard.apply(ImeSignal::Commit));
        guard.apply(ImeSignal::Preedit { has_cursor: true });
        assert!(!guard.apply(ImeSignal::Disabled));
    }
}
