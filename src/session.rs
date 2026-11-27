//! Local window state and transient browsing state; never exported as business data.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowState {
    pub always_on_top: bool,
    pub x: i32,
    pub y: i32,
    pub width: f32,
    pub height: f32,
    pub monitor: String,
    pub edge: Option<u8>,
    pub group: i64,
    pub category: i64,
}

#[derive(Default)]
pub struct ProtectionResume(pub Option<WindowState>);
impl ProtectionResume {
    pub fn enter(&mut self, visible: bool, edge_hide: bool, pinned: bool, state: &WindowState) {
        self.0 = (visible && edge_hide && !pinned && state.edge.is_some()).then(|| state.clone());
    }
    pub fn cancel(&mut self) {
        self.0 = None;
    }
    pub fn take(&mut self, edge_hide: bool, pinned: bool) -> Option<WindowState> {
        self.0.take().filter(|_| edge_hide && !pinned)
    }
}
impl WindowState {
    pub fn valid_geometry(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && (400.0..=10000.0).contains(&self.width)
            && (450.0..=10000.0).contains(&self.height)
            && self.edge.is_none_or(|e| e <= 2)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    All,
    Group,
    Category,
}
impl SearchScope {
    pub fn includes(
        self,
        entry_category: i64,
        collections: &[crate::model::Collection],
        group: i64,
        category: i64,
    ) -> bool {
        match self {
            Self::All => true,
            Self::Category => entry_category == category,
            Self::Group => collections
                .iter()
                .any(|c| c.id == entry_category && c.parent_id == Some(group)),
        }
    }
}
impl From<i32> for SearchScope {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Group,
            2 => Self::Category,
            _ => Self::All,
        }
    }
}
#[derive(Default)]
pub struct Browsing {
    pub views: HashMap<i64, (f32, i32, usize)>,
    pub group_categories: HashMap<i64, i64>,
    pub selected: HashSet<i64>,
    pub anchor: Option<i64>,
}
impl Browsing {
    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }
    pub fn toggle(&mut self, id: i64, extend: bool, ordered: &[i64]) {
        if !ordered.contains(&id) {
            return;
        }
        if extend
            && let Some(a) = self
                .anchor
                .and_then(|a| ordered.iter().position(|i| *i == a))
            && let Some(b) = ordered.iter().position(|i| *i == id)
        {
            self.selected.extend(&ordered[a.min(b)..=a.max(b)]);
        } else {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protection_resume_requires_visible_unpinned_docked_window_and_is_consumed() {
        let state = WindowState {
            width: 440.,
            height: 640.,
            edge: Some(0),
            ..Default::default()
        };
        let mut resume = ProtectionResume::default();
        for (visible, enabled, pinned) in [
            (false, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            resume.enter(visible, enabled, pinned, &state);
            assert!(resume.take(true, false).is_none());
        }
        resume.enter(
            true,
            true,
            false,
            &WindowState {
                edge: None,
                ..state.clone()
            },
        );
        assert!(resume.take(true, false).is_none());
        resume.enter(true, true, false, &state);
        assert_eq!(resume.take(true, false), Some(state.clone()));
        assert!(resume.take(true, false).is_none());
        resume.enter(true, true, false, &state);
        resume.cancel();
        assert!(resume.take(true, false).is_none());
        resume.enter(true, true, false, &state);
        assert!(resume.take(false, false).is_none());
        resume.enter(true, true, false, &state);
        assert!(resume.take(true, true).is_none());
        let old: WindowState = serde_json::from_str(r#"{"width":440,"height":640}"#).unwrap();
        assert!(!old.always_on_top);
    }
    #[test]
    fn search_scope_excludes_sibling_groups_and_stale_categories() {
        let collections = [
            (1, None),
            (2, Some(1)),
            (3, Some(1)),
            (4, None),
            (5, Some(4)),
        ]
        .into_iter()
        .map(|(id, parent_id)| crate::model::Collection {
            id,
            parent_id,
            name: id.to_string(),
            position: id,
        })
        .collect::<Vec<_>>();
        let eligible = |scope: SearchScope| {
            [2, 3, 5]
                .into_iter()
                .filter(|id| scope.includes(*id, &collections, 1, 2))
                .collect::<Vec<_>>()
        };
        assert_eq!(eligible(SearchScope::All), [2, 3, 5]);
        assert_eq!(eligible(SearchScope::Group), [2, 3]);
        assert_eq!(eligible(SearchScope::Category), [2]);
        assert!(!SearchScope::Group.includes(99, &collections, 1, 2));
        assert!(!SearchScope::Group.includes(2, &collections, 99, 2));
    }
    #[test]
    fn ranges_follow_display_order_and_ignore_stale_ids() {
        let mut b = Browsing::default();
        b.toggle(5, false, &[8, 5, 2, 9]);
        b.toggle(9, true, &[8, 5, 2, 9]);
        assert_eq!(b.selected, HashSet::from([5, 2, 9]));
        b.toggle(99, false, &[8, 5, 2, 9]);
        assert_eq!(b.selected.len(), 3);
        b.clear_selection();
        assert!(b.anchor.is_none());
    }
    #[test]
    fn invalid_geometry_is_not_restored() {
        let mut s = WindowState {
            width: 440.,
            height: 640.,
            ..Default::default()
        };
        assert!(s.valid_geometry());
        s.width = f32::NAN;
        assert!(!s.valid_geometry());
    }
}

/// Interval selection is independent of rendering and has no hidden-window wakeups.
/// `pointer_far` halves the docked rate while the cursor is nowhere near the collapsed strip;
/// crossing the approach margin puts the fine rate back before the reveal dwell can elapse.
pub fn interaction_interval(
    visible: bool,
    animated: bool,
    edge_or_drag: bool,
    pointer_far: bool,
) -> Option<u64> {
    if !visible {
        None
    } else if animated {
        Some(8)
    } else if edge_or_drag {
        Some(if pointer_far { 32 } else { 16 })
    } else {
        None
    }
}
/// Notices raised while protection is active cannot be shown, and re-showing them all later
/// would stack in a single-row strip. Identical text moves to the back instead of duplicating.
#[derive(Default)]
pub struct NoticeQueue {
    items: std::collections::VecDeque<String>,
    dropped: usize,
}
impl NoticeQueue {
    const LIMIT: usize = 8;
    pub fn push(&mut self, text: String) {
        if let Some(index) = self.items.iter().position(|kept| *kept == text) {
            if let Some(kept) = self.items.remove(index) {
                self.items.push_back(kept);
            }
            return;
        }
        if self.items.len() == Self::LIMIT {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back(text);
    }
    /// Returns the most recent notice with a count of what it stands in for, and clears the queue.
    pub fn take(&mut self) -> String {
        self.take_for(crate::i18n::locale())
    }

    pub fn take_for(&mut self, locale: crate::i18n::Locale) -> String {
        let Some(text) = self.items.pop_back() else {
            self.dropped = 0;
            return String::new();
        };
        let mut text = text;
        let more = std::mem::take(&mut self.items).len();
        if more > 0 {
            text.push_str(&match locale {
                crate::i18n::Locale::ZhCn => format!("（另有 {more} 条提示）"),
                crate::i18n::Locale::EnUs => format!(" (+{more} more)"),
            });
        }
        if self.dropped > 0 {
            text.push_str(&match locale {
                crate::i18n::Locale::ZhCn => format!("，已省略 {} 条", self.dropped),
                crate::i18n::Locale::EnUs => format!(", {} omitted", self.dropped),
            });
            self.dropped = 0;
        }
        text
    }
}
#[cfg(test)]
mod scheduling_tests {
    #[test]
    fn notices_coalesce_replace_and_lose_the_oldest_when_full() {
        use super::NoticeQueue;
        let mut queue = NoticeQueue::default();
        assert_eq!(queue.take(), "");
        queue.push("第一条".into());
        queue.push("第一条".into());
        assert_eq!(queue.take(), "第一条", "repeats do not duplicate");
        for index in 0..10 {
            queue.push(format!("提示{index}"));
        }
        let flushed = queue.take();
        assert!(flushed.starts_with("提示9"), "{flushed}");
        assert!(flushed.contains("另有 7 条"), "{flushed}");
        assert!(flushed.contains("已省略 2 条"), "{flushed}");
        assert_eq!(queue.take(), "", "one flush drains the queue");
    }
    #[test]
    fn hidden_is_stopped_and_motion_has_priority() {
        use super::interaction_interval as interval;
        assert_eq!(interval(false, true, true, false), None);
        assert_eq!(interval(true, true, true, false), Some(8));
        assert_eq!(interval(true, false, true, false), Some(16));
        assert_eq!(interval(true, false, true, true), Some(32));
        assert_eq!(interval(true, false, false, true), None);
    }
}
