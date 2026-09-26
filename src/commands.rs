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
    /// Validate the complete resulting data before the transaction, through the same identity the
    /// `dup_key` column is written from. This used to re-derive it locally, which let a move pass a
    /// check whose own write then disagreed.
    fn commit_commands(&mut self, data: &Export) -> Result<()> {
        if let Some(name) = self.export_clash(&data.entries) {
            return Err(Error::Invalid(format!("目标分类已有重复项目：{name}")));
        }
        self.restore(data)
    }
    pub fn convert_entries(&mut self, preview: &ConversionPreview) -> Result<()> {
        // Compare inside the transaction; stale previews never overwrite concurrent changes.
        self.update_targets_checked(&preview.entries, &preview.selected)
    }
    pub fn reorder_collection(&mut self, id: i64, before: i64) -> Result<()> {
        self.reorder_collection_at(id, before, false)
    }
    pub fn reorder_collection_at(&mut self, id: i64, before: i64, after: bool) -> Result<()> {
        let mut data = self.export()?;
        let source = data
            .collections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Invalid("分组不存在".into()))?;
        let parent = source.parent_id;
        if !data
            .collections
            .iter()
            .any(|c| c.id == before && c.parent_id == parent)
        {
            return Err(Error::Invalid("只能在同一级排序".into()));
        }
        if id == before {
            return Ok(());
        }
        let mut ids: Vec<_> = data
            .collections
            .iter()
            .filter(|c| c.parent_id == parent && c.id != id)
            .map(|c| c.id)
            .collect();
        let at = ids.iter().position(|c| *c == before).unwrap();
        ids.insert(at + usize::from(after), id);
        for (position, id) in ids.iter().enumerate() {
            data.collections
                .iter_mut()
                .find(|c| c.id == *id)
                .unwrap()
                .position = position as i64;
        }
        self.restore(&data)
    }
    pub fn move_to_collection(&mut self, id: i64, destination: i64) -> Result<()> {
        let mut data = self.export()?;
        let end = data
            .entries
            .iter()
            .filter(|e| e.collection_id == destination)
            .map(|e| e.position)
            .max()
            .unwrap_or(-1)
            + 1;
        let e = data
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::Invalid("项目不存在".into()))?;
        if e.collection_id == destination {
            return Ok(());
        }
        e.collection_id = destination;
        e.position = end;
        self.commit_commands(&data)
    }
    pub fn migrate_delete(&mut self, id: i64, destination: i64) -> Result<()> {
        let mut data = self.export()?;
        let source = data
            .collections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Invalid("分组不存在".into()))?
            .clone();
        let dest = data
            .collections
            .iter()
            .find(|c| c.id == destination)
            .ok_or_else(|| Error::Invalid("请选择迁移目标".into()))?
            .clone();
        if id == destination
            || dest.parent_id == Some(id)
            || (source.parent_id.is_none() && dest.parent_id.is_some())
        {
            return Err(Error::Invalid("迁移目标无效".into()));
        }
        let end = data
            .entries
            .iter()
            .filter(|e| e.collection_id == destination)
            .map(|e| e.position)
            .max()
            .unwrap_or(-1)
            + 1;
        for (offset, e) in data
            .entries
            .iter_mut()
            .filter(|e| e.collection_id == id)
            .enumerate()
        {
            e.collection_id = destination;
            e.position = end + offset as i64;
        }
        let next = data
            .collections
            .iter()
            .filter(|c| c.parent_id == Some(destination))
            .map(|c| c.position)
            .max()
            .unwrap_or(-1)
            + 1;
        // Nothing is written before commit_commands, so the names already claimed under the
        // destination — including ones this very loop placed — are only knowable from memory.
        let mut taken: HashSet<String> = data
            .collections
            .iter()
            .filter(|c| c.parent_id == Some(destination))
            .map(|c| c.name.trim().to_lowercase())
            .collect();
        let mut dropped = Vec::new();
        let mut moved: i64 = 0;
        for c in data
            .collections
            .iter_mut()
            .filter(|c| c.parent_id == Some(id))
        {
            let stem = c.name.clone();
            let child = c.id;
            // 默认分类 is a placeholder this app creates by itself, so an empty one is not
            // something to relocate and keep: renaming it would strand a `默认分类 2` row that
            // normalize_categories never reuses.
            if stem == "默认分类"
                && taken.contains("默认分类")
                && !data.entries.iter().any(|e| e.collection_id == child)
            {
                dropped.push(child);
                continue;
            }
            let mut name = stem.clone();
            let mut suffix = 2;
            while taken.contains(&name.trim().to_lowercase()) {
                name = format!("{stem} {suffix}");
                suffix += 1;
            }
            taken.insert(name.trim().to_lowercase());
            c.name = name;
            c.parent_id = Some(destination);
            c.position = next + moved;
            moved += 1;
        }
        data.collections
            .retain(|c| c.id != id && !dropped.contains(&c.id));
        self.commit_commands(&data)
    }
    pub fn sort_names(&mut self, selected: &HashSet<i64>) -> Result<()> {
        let mut data = self.export()?;
        let mut entries: Vec<_> = data
            .entries
            .iter_mut()
            .filter(|e| selected.contains(&e.collection_id))
            .collect();
        entries.sort_by_key(|e| (e.name.to_lowercase(), e.id));
        for (position, e) in entries.into_iter().enumerate() {
            e.position = position as i64;
        }
        self.restore(&data)
    }
}
