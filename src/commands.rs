//! Shared collection and conversion commands. UI menus never mutate models directly.
use crate::{model::*, paths, storage::Store};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionPreview {
    pub entries: Vec<Entry>,
    pub unchanged: usize,
    pub skipped: Vec<String>,
    pub selected: Vec<Entry>,
}
pub fn conversion_preview(
    entries: &[Entry],
    selected: &HashSet<i64>,
    base: &Path,
    relative: bool,
) -> ConversionPreview {
    let mut out = ConversionPreview {
        entries: vec![],
        unchanged: 0,
        skipped: vec![],
        selected: vec![],
    };
    for e in entries
        .iter()
        .filter(|e| selected.contains(&e.collection_id))
    {
        out.selected.push(e.clone());
        match paths::convert_target(&e.target, base, relative) {
            Ok(target) if target == e.target => out.unchanged += 1,
            Ok(target) => {
                let mut entry = e.clone();
                entry.target = target;
                out.entries.push(entry);
            }
            Err(error) => out.skipped.push(format!("{}：{error}", e.name)),
        }
    }
    out
}
impl Store {
    pub fn convert_entries(&mut self, preview: &ConversionPreview) -> Result<()> {
        // Compare inside the transaction; stale previews never overwrite concurrent changes.
        self.update_targets_checked(&preview.entries, &preview.selected)
    }
    pub fn reorder_collection(&mut self, id: i64, before: i64) -> Result<()> {
        self.reorder_collection_at(id, before, false)
    }
}
