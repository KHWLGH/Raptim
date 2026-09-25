use crate::model::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

pub struct Store {
    conn: Connection,
    base: std::path::PathBuf,
    initial: Option<Export>,
}
const SCHEMA_VERSION: u32 = 2;
const SCHEMA_DDL: &str = "CREATE TABLE collections (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES collections(id) ON DELETE RESTRICT, name TEXT NOT NULL, position INTEGER NOT NULL);
                CREATE TABLE entries (id INTEGER PRIMARY KEY, collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE RESTRICT, payload TEXT NOT NULL, position INTEGER NOT NULL, icon BLOB, dup_key TEXT NOT NULL);
                CREATE INDEX entries_collection ON entries(collection_id, dup_key);
                CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);
                CREATE TABLE local_state (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);
                INSERT INTO collections VALUES (1, NULL, '常用', 0), (2, 1, '默认分类', 0);
                PRAGMA user_version=2;";
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            let tables: i64 =
                conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |row| row.get(0))?;
            if tables != 0 {
                // Diascia restarts its version counter, so a foreign version number cannot tell a
                // future build from a pre-reset one; the table set is what separates the wording.
                let ours: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('collections','entries','settings','local_state')", [], |row| row.get(0))?;
                return Err(Error::Invalid(if ours == 4 {
                    format!("该数据库不是当前版本支持的格式（user_version={version}）；未修改数据")
                } else {
                    "无法识别该数据库；未修改数据".into()
                }));
            }
            let tx = conn.transaction()?;
            tx.execute_batch(SCHEMA_DDL)?;
            tx.execute(
                "INSERT INTO settings VALUES(1, ?1)",
                [serde_json::to_string(&Settings::default())?],
            )?;
            tx.commit()?;
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        // Measured, not assumed: capping the page cache at 4 MiB (`cache_size=-4000`) doubled the
        // startup table read from 27.8 to 56.5 ms, because the icon BLOBs alone exceed it. SQLite's
        // 12.5 MiB default stays.
        let base = std::path::absolute(path)?.parent().unwrap().to_path_buf();
        let mut store = Self {
            conn,
            base,
            initial: None,
        };
        // Fail visibly on damaged payload/settings. Never silently replace user data.
        let started = Instant::now();
        let validated = store.export()?;
        crate::metrics::record("store_export_ms", started.elapsed().as_secs_f64() * 1000.);
        let started = Instant::now();
        validate_export(&validated)?;
        crate::metrics::record("store_validate_ms", started.elapsed().as_secs_f64() * 1000.);
        store.initial = Some(validated);
        Ok(store)
    }
    pub fn take_initial(&mut self) -> Result<Export> {
        match self.initial.take() {
            Some(data) => Ok(data),
            None => self.export(),
        }
    }
    pub fn window_state(&self) -> Result<crate::session::WindowState> {
        let text: Option<String> = self
            .conn
            .query_row("SELECT payload FROM local_state WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(text
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }
    pub fn save_window_state(&mut self, state: &crate::session::WindowState) -> Result<()> {
        if !state.valid_geometry() {
            return Err(Error::Invalid("窗口几何无效".into()));
        }
        self.conn.execute("INSERT INTO local_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(state)?])?;
        Ok(())
    }
    /// IDs are supplied in display order. All validation completes before any write.
    pub fn batch_entries(
        &mut self,
        ids: &[i64],
        destination: Option<i64>,
        copy: bool,
    ) -> Result<()> {
        if ids.is_empty() {
            return Err(Error::Invalid("请先选择项目".into()));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let all = self.entries()?;
        let mut seen = HashSet::new();
        let mut selected = Vec::new();
        for id in ids {
            if !seen.insert(*id) {
                return Err(Error::Invalid("选择包含重复项目".into()));
            }
            selected.push(
                all.iter()
                    .find(|e| e.id == *id)
                    .ok_or_else(|| Error::Invalid("项目已变化，请重新选择".into()))?,
            );
        }
        if let Some(dest) = destination {
            if !self
                .collections()?
                .iter()
                .any(|c| c.id == dest && c.parent_id.is_some())
            {
                return Err(Error::Invalid("目标分类已不存在".into()));
            }
            let mut keys: HashSet<_> = all
                .iter()
                .filter(|e| e.collection_id == dest)
                .map(|e| self.target_key(&e.target))
                .collect();
            for e in &selected {
                if !keys.insert(self.target_key(&e.target)) {
                    return Err(Error::Invalid(format!(
                        "目标分类存在重复项目：{}；整批未修改",
                        e.name
                    )));
                }
            }
        }

        let mut position: i64 = tx.query_row(
            "SELECT coalesce(max(position),-1)+1 FROM entries",
            [],
            |r| r.get(0),
        )?;
        for entry in selected {
            if let Some(dest) = destination {
                let mut updated = entry.clone();
                updated.collection_id = dest;
                updated.position = position;
                let payload = Self::payload_of(&updated)?;
                let dup_key = self.dup_key(&updated);
                if copy {
                    updated.id = 0;
                    tx.execute(
                        "INSERT INTO entries(collection_id,payload,position,icon,dup_key) VALUES(?1,?2,?3,?4,?5)",
                        params![dest, payload, position, updated.icon.as_slice(), dup_key],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE entries SET collection_id=?1,payload=?2,position=?3,icon=?4,dup_key=?5 WHERE id=?6",
                        params![
                            dest,
                            payload,
                            position,
                            updated.icon.as_slice(),
                            dup_key,
                            entry.id
                        ],
                    )?;
                }
                position += 1;
            } else {
                tx.execute("DELETE FROM entries WHERE id=?1", [entry.id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Guards the "every group owns at least one real category, no entry sits on a group"
    /// invariant, which the write paths that accept an arbitrary collection id rely on.
    pub fn ensure_categories(&mut self) -> Result<()> {
        let needs: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM collections g WHERE g.parent_id IS NULL AND (NOT EXISTS(SELECT 1 FROM collections c WHERE c.parent_id=g.id) OR EXISTS(SELECT 1 FROM entries e WHERE e.collection_id=g.id)))", [], |r| r.get(0))?;
        if needs {
            let tx = self.conn.transaction()?;
            normalize_categories(&tx)?;
            tx.commit()?;
        }
        Ok(())
    }
    fn target_key(&self, target: &Target) -> String {
        crate::paths::convert_target(target, &self.base, false)
            .unwrap_or_else(|_| target.clone())
            .duplicate_key()
    }
    /// The `payload` column: the entry without its icon, which lives in the `icon` BLOB column so
    /// that reading a library never decodes icon bytes. A version 1 database kept both in one JSON
    /// document, where `Vec<u8>` costs ~3.6 text characters per byte and one token each.
    fn payload_of(entry: &Entry) -> Result<String> {
        let mut row = entry.clone();
        row.icon = Vec::new();
        Ok(serde_json::to_string(&row)?)
    }
    /// The dedup identity, materialised because `target_key` relativises paths against the data
    /// directory and lower-cases URLs, neither of which SQLite can express. It exists so the
    /// duplicate-target check is an index probe instead of a full-table re-read.
    fn dup_key(&self, entry: &Entry) -> String {
        self.target_key(&entry.target)
    }
    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut statement = self
            .conn
            .prepare("SELECT id,parent_id,name,position FROM collections ORDER BY position,id")?;
        Ok(statement
            .query_map([], |r| {
                Ok(Collection {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    name: r.get(2)?,
                    position: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let started = Instant::now();
        let mut statement = self.conn.prepare(
            "SELECT id,collection_id,payload,position,icon FROM entries ORDER BY position,id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<Vec<u8>>>(4)?,
            ))
        })?;
        let entries = rows
            .map(|row| {
                let (id, collection_id, payload, position, icon) = row?;
                let mut e: Entry = serde_json::from_str(&payload)?;
                e.id = id;
                e.collection_id = collection_id;
                e.position = position;
                e.icon = icon.unwrap_or_default();
                Ok(e)
            })
            .collect();
        crate::metrics::record("entries_query_ms", started.elapsed().as_secs_f64() * 1000.);
        entries
    }
    pub fn settings(&self) -> Result<Settings> {
        let json: String =
            self.conn
                .query_row("SELECT payload FROM settings WHERE id=1", [], |r| r.get(0))?;
        Ok(serde_json::from_str(&json)?)
    }
    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        validate_settings(settings)?;
        self.conn.execute(
            "UPDATE settings SET payload=?1 WHERE id=1",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }
    pub fn update_targets_checked(&mut self, updates: &[Entry], originals: &[Entry]) -> Result<()> {
        // Resolved before the transaction because it needs `&self`, which the write borrow denies.
        let keys: Vec<String> = updates.iter().map(|e| self.dup_key(e)).collect();
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for old in originals {
            let row: (i64, String, i64, Option<Vec<u8>>) = tx.query_row(
                "SELECT collection_id,payload,position,icon FROM entries WHERE id=?1",
                [old.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let mut entry: Entry = serde_json::from_str(&row.1)?;
            entry.id = old.id;
            entry.collection_id = row.0;
            entry.position = row.2;
            entry.icon = row.3.unwrap_or_default();
            if &entry != old {
                return Err(Error::Invalid("项目已变化，请重新预览".into()));
            }
        }
        for (e, dup_key) in updates.iter().zip(keys) {
            e.validate()?;
            tx.execute(
                "UPDATE entries SET payload=?1,icon=?2,dup_key=?3 WHERE id=?4",
                params![Self::payload_of(e)?, e.icon.as_slice(), dup_key, e.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn save_collection(
        &mut self,
        id: Option<i64>,
        parent_id: Option<i64>,
        name: &str,
    ) -> Result<i64> {
        validate_name(name)?;
        let all = self.collections()?;
        if let Some(parent) = parent_id {
            if !all.iter().any(|c| c.id == parent && c.parent_id.is_none()) || id == Some(parent) {
                return Err(Error::Invalid("分类只能放在一级分组下".into()));
            }
            if id.is_some_and(|id| all.iter().any(|c| c.parent_id == Some(id))) {
                return Err(Error::Invalid("请先移动该分组下的分类".into()));
            }
        }
        if all.iter().any(|c| {
            Some(c.id) != id
                && c.parent_id == parent_id
                && c.name.to_lowercase() == name.trim().to_lowercase()
        }) {
            return Err(Error::Invalid("同一位置已有同名分组或分类".into()));
        }
        if let Some(id) = id {
            let tx = self.conn.transaction()?;
            if tx.execute(
                "UPDATE collections SET parent_id=?1,name=?2 WHERE id=?3",
                params![parent_id, name.trim(), id],
            )? == 0
            {
                return Err(Error::Invalid("分组已不存在".into()));
            }
            normalize_categories(&tx)?;
            tx.commit()?;
            Ok(id)
        } else {
            let tx = self.conn.transaction()?;
            tx.execute("INSERT INTO collections(parent_id,name,position) VALUES(?1,?2,(SELECT coalesce(max(position),-1)+1 FROM collections))", params![parent_id, name.trim()])?;
            let id = tx.last_insert_rowid();
            if parent_id.is_none() {
                tx.execute(
                    "INSERT INTO collections(parent_id,name,position) VALUES(?1,'默认分类',0)",
                    [id],
                )?;
            }
            tx.commit()?;
            Ok(id)
        }
    }
    pub fn delete_collection(&mut self, id: i64) -> Result<()> {
        let groups = self.collections()?;
        if groups.iter().any(|c| c.parent_id == Some(id))
            || self.entries()?.iter().any(|e| e.collection_id == id)
        {
            return Err(Error::Invalid("请先移走项目和子分类，再删除空分组".into()));
        }
        if groups.iter().filter(|c| c.parent_id.is_none()).count() == 1
            && groups.iter().any(|c| c.id == id && c.parent_id.is_none())
        {
            return Err(Error::Invalid("至少保留一个分组".into()));
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM collections WHERE id=?1", [id])?;
        normalize_categories(&tx)?;
        tx.commit()?;
        Ok(())
    }
    /// Delete a collection together with what hangs off it. The tree is two levels deep, so a
    /// group's children and their entries are everything else that has to go first.
    pub fn delete_collection_with_contents(&mut self, id: i64) -> Result<()> {
        let collections = self.collections()?;
        if collections.iter().filter(|c| c.parent_id.is_none()).count() == 1
            && collections
                .iter()
                .any(|c| c.id == id && c.parent_id.is_none())
        {
            return Err(Error::Invalid("至少保留一个分组".into()));
        }
        let ids: Vec<i64> = collections
            .iter()
            .filter(|c| c.id == id || c.parent_id == Some(id))
            .map(|c| c.id)
            .collect();
        let tx = self.conn.transaction()?;
        for child in ids {
            tx.execute("DELETE FROM entries WHERE collection_id=?1", [child])?;
        }
        tx.execute("DELETE FROM collections WHERE parent_id=?1", [id])?;
        tx.execute("DELETE FROM collections WHERE id=?1", [id])?;
        normalize_categories(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn save_entry(&mut self, entry: &Entry) -> Result<i64> {
        let started = Instant::now();
        entry.validate()?;
        let payload = Self::payload_of(entry)?;
        let dup_key = self.dup_key(entry);
        let clash: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE collection_id=?1 AND dup_key=?2 AND id<>?3)",
            params![entry.collection_id, dup_key, entry.id],
            |r| r.get(0),
        )?;
        if clash {
            return Err(Error::Invalid("当前分类已收录同一目标".into()));
        }
        let id = if entry.id == 0 {
            self.conn.execute("INSERT INTO entries(collection_id,payload,position,icon,dup_key) VALUES(?1,?2,(SELECT coalesce(max(position),-1)+1 FROM entries),?3,?4)", params![entry.collection_id, payload, entry.icon.as_slice(), dup_key])?;
            self.conn.last_insert_rowid()
        } else if self.conn.execute(
            "UPDATE entries SET collection_id=?1,payload=?2,icon=?3,dup_key=?4 WHERE id=?5",
            params![
                entry.collection_id,
                payload,
                entry.icon.as_slice(),
                dup_key,
                entry.id
            ],
        )? == 0
        {
            return Err(Error::Invalid("项目已不存在".into()));
        } else {
            entry.id
        };
        crate::metrics::record("save_entry_ms", started.elapsed().as_secs_f64() * 1000.);
        Ok(id)
    }
    /// Only the icon column. The backfill used to go through `save_entry`, which rewrote the
    /// payload and re-ran the whole duplicate check for a change that touches neither.
    pub fn save_icon(&mut self, id: i64, icon: &[u8]) -> Result<()> {
        if self
            .conn
            .execute("UPDATE entries SET icon=?2 WHERE id=?1", params![id, icon])?
            == 0
        {
            return Err(Error::Invalid("项目已不存在".into()));
        }
        Ok(())
    }
    pub fn delete_entry(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM entries WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn copy_to_collection(&mut self, id: i64, destination: i64) -> Result<i64> {
        if !self
            .collections()?
            .iter()
            .any(|c| c.id == destination && c.parent_id.is_some())
        {
            return Err(Error::Invalid("目标分类已不存在".into()));
        }
        let row = self
            .conn
            .query_row("SELECT payload,icon FROM entries WHERE id=?1", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<Vec<u8>>>(1)?))
            })
            .optional()?;
        let Some((payload, icon)) = row else {
            return Err(Error::Invalid("项目已不存在".into()));
        };
        let mut entry: Entry = serde_json::from_str(&payload)?;
        entry.icon = icon.unwrap_or_default();
        entry.id = 0;
        entry.collection_id = destination;
        self.save_entry(&entry)
    }
    /// One drop/picker operation commits atomically; repeated targets are skipped.
    pub fn add_entries(&mut self, entries: &[Entry]) -> Result<(usize, usize)> {
        // Only the two dedup columns are read. The previous shape re-parsed every payload, icon
        // bytes included, to answer "is this target already here".
        let mut keys: HashSet<(i64, String)> = self
            .conn
            .prepare("SELECT collection_id,dup_key FROM entries")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut rows = Vec::new();
        for entry in entries {
            entry.validate()?;
            let dup_key = self.dup_key(entry);
            if keys.insert((entry.collection_id, dup_key.clone())) {
                rows.push((entry, Self::payload_of(entry)?, dup_key));
            }
        }
        let tx = self.conn.transaction()?;
        for (entry, payload, dup_key) in &rows {
            tx.execute("INSERT INTO entries(collection_id,payload,position,icon,dup_key) VALUES(?1,?2,(SELECT coalesce(max(position),-1)+1 FROM entries),?3,?4)", params![entry.collection_id, payload, entry.icon.as_slice(), dup_key])?;
        }
        tx.commit()?;
        let accepted = rows.len();
        Ok((accepted, entries.len() - accepted))
    }
    pub fn move_at(&mut self, id: i64, before: i64, after: bool) -> Result<()> {
        let all = self.entries()?;
        let source = all
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::Invalid("项目已不存在".into()))?;
        if !all
            .iter()
            .any(|e| e.id == before && e.collection_id == source.collection_id)
        {
            return Err(Error::Invalid("只能在当前分类内排序".into()));
        }
        if id == before {
            return Ok(());
        }
        let mut order: Vec<_> = all
            .iter()
            .filter(|e| e.collection_id == source.collection_id && e.id != id)
            .map(|e| e.id)
            .collect();
        let index = order.iter().position(|e| *e == before).unwrap();
        order.insert(index + usize::from(after), id);
        let tx = self.conn.transaction()?;
        for (position, id) in order.iter().enumerate() {
            tx.execute(
                "UPDATE entries SET position=?1 WHERE id=?2",
                params![position as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn move_entry(&mut self, id: i64, direction: i32) -> Result<()> {
        let all = self.entries()?;
        let Some(entry) = all.iter().find(|e| e.id == id) else {
            return Err(Error::Invalid("项目已不存在".into()));
        };
        let mut siblings: Vec<_> = all
            .iter()
            .filter(|e| e.collection_id == entry.collection_id)
            .collect();
        let index = siblings.iter().position(|e| e.id == id).unwrap();
        let next = index as i64 + i64::from(direction.signum());
        if next < 0 || next as usize >= siblings.len() {
            return Ok(());
        }
        siblings.swap(index, next as usize);
        let tx = self.conn.transaction()?;
        for (position, e) in siblings.iter().enumerate() {
            tx.execute(
                "UPDATE entries SET position=?1 WHERE id=?2",
                params![position as i64, e.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn export(&self) -> Result<Export> {
        Ok(Export {
            format_version: SCHEMA_VERSION,
            collections: self.collections()?,
            entries: self.entries()?,
            settings: self.settings()?,
        })
    }
    /// SQLite's backup API includes committed WAL data; copying just the .db would not.
    pub fn backup(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(Error::Invalid("备份文件已存在，请换一个名称".into()));
        }
        let mut target = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut target)?;
        backup.run_to_completion(64, Duration::from_millis(5), None)?;
        Ok(())
    }
    pub fn restore(&mut self, data: &Export) -> Result<()> {
        validate_export(data)?;
        // Resolved while `&self` is still free: the transaction holds it mutably afterwards.
        let keys: Vec<String> = data.entries.iter().map(|e| self.dup_key(e)).collect();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM entries", [])?;
        tx.execute("DELETE FROM collections WHERE parent_id IS NOT NULL", [])?;
        tx.execute("DELETE FROM collections", [])?;
        for collection in data
            .collections
            .iter()
            .filter(|c| c.parent_id.is_none())
            .chain(data.collections.iter().filter(|c| c.parent_id.is_some()))
        {
            tx.execute(
                "INSERT INTO collections VALUES(?1,?2,?3,?4)",
                params![
                    collection.id,
                    collection.parent_id,
                    collection.name,
                    collection.position
                ],
            )?;
        }
        for (entry, dup_key) in data.entries.iter().zip(keys) {
            tx.execute(
                "INSERT INTO entries(id,collection_id,payload,position,icon,dup_key) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    entry.id,
                    entry.collection_id,
                    Self::payload_of(entry)?,
                    entry.position,
                    entry.icon.as_slice(),
                    dup_key
                ],
            )?;
        }
        tx.execute(
            "UPDATE settings SET payload=?1 WHERE id=1",
            [serde_json::to_string(&data.settings)?],
        )?;
        normalize_categories(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn has_entry(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT id FROM entries WHERE id=?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .is_some())
    }
}
fn normalize_categories(conn: &Connection) -> Result<()> {
    let groups: Vec<i64> = conn
        .prepare("SELECT id FROM collections WHERE parent_id IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for group in groups {
        let children: i64 = conn.query_row(
            "SELECT count(*) FROM collections WHERE parent_id=?1",
            [group],
            |r| r.get(0),
        )?;
        let direct: i64 = conn.query_row(
            "SELECT count(*) FROM entries WHERE collection_id=?1",
            [group],
            |r| r.get(0),
        )?;
        if children > 0 && direct == 0 {
            continue;
        }
        let empty_default: Option<i64> = conn.query_row(
            "SELECT id FROM collections c WHERE parent_id=?1 AND name='默认分类' AND NOT EXISTS(SELECT 1 FROM entries e WHERE e.collection_id=c.id)",
            [group], |r| r.get(0),
        ).optional()?;
        if let Some(category) = empty_default {
            conn.execute(
                "UPDATE entries SET collection_id=?1 WHERE collection_id=?2",
                params![category, group],
            )?;
            continue;
        }
        let mut name = "默认分类".to_string();
        let mut suffix = 2;
        while conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE parent_id=?1 AND name=?2)",
            params![group, name],
            |r| r.get::<_, bool>(0),
        )? {
            name = format!("默认分类 {suffix}");
            suffix += 1;
        }
        conn.execute("INSERT INTO collections(parent_id,name,position) VALUES(?1,?2,(SELECT coalesce(max(position),-1)+1 FROM collections))", params![group,name])?;
        let category = conn.last_insert_rowid();
        conn.execute(
            "UPDATE entries SET collection_id=?1 WHERE collection_id=?2",
            params![category, group],
        )?;
    }
    Ok(())
}
pub fn validate_settings(settings: &Settings) -> Result<()> {
    if settings.icon_size > 2 {
        return Err(Error::Invalid("图标大小必须为小、中或大".into()));
    }
    crate::hotkey::parse(&settings.hotkey).map_err(Error::Invalid)?;
    if !(1..=12).contains(&settings.edge_strip_width_px) {
        return Err(Error::Invalid("露边宽度应为 1–12 个物理像素".into()));
    }
    if !(100..=2000).contains(&settings.hover_delay_ms) {
        return Err(Error::Invalid("悬停延迟应在 100–2000 ms 之间".into()));
    }
    if settings.protected_apps.len() > 100 {
        return Err(Error::Invalid("应用规则最多 100 项".into()));
    }
    for path in &settings.protected_apps {
        Target::File { path: path.clone() }.validate()?;
    }
    Ok(())
}
pub fn validate_export(data: &Export) -> Result<()> {
    if data.format_version != SCHEMA_VERSION {
        return Err(Error::Invalid(format!(
            "此导出文件为版本 {}，Diascia 只能恢复版本 {SCHEMA_VERSION} 的导出",
            data.format_version
        )));
    }
    if data.collections.len() > 10000 || data.entries.len() > 100000 {
        return Err(Error::Invalid("数据量超出当前版本限制".into()));
    }
    if !data.collections.iter().any(|c| c.parent_id.is_none()) {
        return Err(Error::Invalid("至少需要一个一级分组".into()));
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for c in &data.collections {
        validate_name(&c.name)?;
        if c.id <= 0
            || c.id > i32::MAX as i64
            || !ids.insert(c.id)
            || !names.insert((c.parent_id, c.name.trim().to_lowercase()))
        {
            return Err(Error::Invalid("分组编号或名称重复/无效".into()));
        }
        if let Some(parent) = c.parent_id
            && (parent == c.id
                || !data
                    .collections
                    .iter()
                    .any(|p| p.id == parent && p.parent_id.is_none()))
        {
            return Err(Error::Invalid("分组关系无效或超过两级".into()));
        }
    }
    let mut entries = HashSet::new();
    let mut targets = HashSet::new();
    for e in &data.entries {
        e.validate()?;
        if e.id <= 0
            || e.id > i32::MAX as i64
            || !entries.insert(e.id)
            || !ids.contains(&e.collection_id)
        {
            return Err(Error::Invalid("项目编号重复或分类不存在".into()));
        }
        if !targets.insert((e.collection_id, e.target.duplicate_key())) {
            return Err(Error::Invalid("同一分类存在重复目标".into()));
        }
    }
    validate_settings(&data.settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        Entry {
            icon: Vec::new(),
            id: 0,
            collection_id: 2,
            name: "示例".into(),
            aliases: "example".into(),
            notes: String::new(),
            target: Target::Url {
                url: "https://example.com".into(),
            },
            position: 0,
        }
    }
    /// The reason for the split columns: reading a library must not touch icon bytes at all, and
    /// an icon must not cost ~3.6 text characters per byte wherever it is serialised.
    #[test]
    fn icon_bytes_live_in_their_own_column_not_in_the_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.db");
        let mut store = Store::open(&path).unwrap();
        let mut e = entry();
        e.icon = vec![7u8; 100 * 1024];
        let id = store.save_entry(&e).unwrap();
        let (payload_len, icon_len): (i64, i64) = store
            .conn
            .query_row(
                "SELECT length(payload),length(icon) FROM entries WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(icon_len, 100 * 1024);
        assert!(
            payload_len < 2048,
            "payload still carries the icon: {payload_len} bytes"
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.entries().unwrap()[0].icon, vec![7u8; 100 * 1024]);
        // The JSON export keeps icons, but base64 costs about 1.33 characters per byte instead of
        // one whole number token each.
        let json = serde_json::to_string(&store.export().unwrap()).unwrap();
        // base64 of 0x07 0x07 0x07.
        assert!(json.contains("BwcH"), "export lost the icon");
        assert!(
            json.len() < 200_000,
            "export still spells bytes as integers: {} chars",
            json.len()
        );
    }
    #[test]
    fn duplicate_target_is_rejected_after_a_category_change_too() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.save_entry(&entry()).unwrap();
        assert!(store.save_entry(&entry()).is_err());
        // The rule is per category, so the same target in a sibling category is legal.
        let other = store.save_collection(None, Some(1), "另一个分类").unwrap();
        let mut twin = entry();
        twin.collection_id = other;
        let twin_id = store.save_entry(&twin).unwrap();
        // Moving the first one over there now collides with that twin.
        let mut moved = entry();
        moved.id = id;
        moved.collection_id = other;
        assert!(store.save_entry(&moved).is_err());
        // With the twin gone the move is legal again, which is what the dedup index must track.
        store.delete_entry(twin_id).unwrap();
        assert_eq!(store.save_entry(&moved).unwrap(), id);
    }
    #[test]
    fn save_icon_touches_only_the_icon_column() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.save_entry(&entry()).unwrap();
        let before: (String, String) = store
            .conn
            .query_row(
                "SELECT payload,dup_key FROM entries WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        store.save_icon(id, &[9u8; 4096]).unwrap();
        let after: (String, String) = store
            .conn
            .query_row(
                "SELECT payload,dup_key FROM entries WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            before, after,
            "the backfill must not rewrite payload or dup_key"
        );
        assert_eq!(store.entries().unwrap()[0].icon, vec![9u8; 4096]);
        // The dedup probe still works on a row whose icon was replaced behind its payload.
        assert!(store.save_entry(&entry()).is_err());
        assert!(store.save_icon(4040, &[1]).is_err());
    }
    #[test]
    fn add_entries_still_skips_targets_already_present() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.save_entry(&entry()).unwrap();
        let mut fresh = entry();
        fresh.target = Target::Url {
            url: "https://other.example".into(),
        };
        assert_eq!(
            store.add_entries(&[entry(), fresh]).unwrap(),
            (1, 1),
            "the dedup set now comes from the dup_key column"
        );
    }
    fn open_error(path: &Path) -> String {
        match Store::open(path) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("database must not be opened"),
        }
    }
    #[test]
    fn window_state_never_enters_export_and_survives_restore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut store = Store::open(&path).unwrap();
        store.save_entry(&entry()).unwrap();
        let state = crate::session::WindowState {
            always_on_top: true,
            x: -500,
            y: 40,
            width: 440.,
            height: 640.,
            monitor: "second".into(),
            edge: Some(1),
            group: 1,
            category: 2,
        };
        store.save_window_state(&state).unwrap();
        let export = store.export().unwrap();
        let json = serde_json::to_string(&export).unwrap();
        assert_eq!(export.format_version, 2);
        assert!(!json.contains("monitor"));
        assert!(!json.contains("always_on_top"));
        store.restore(&export).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.window_state().unwrap(), state);
        assert_eq!(
            store
                .conn
                .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|p| p.ok())
                .filter(|p| p.path().extension().is_some_and(|e| e == "db"))
                .count(),
            1,
            "opening a database must never copy or rewrite files beside it"
        );
    }
    #[test]
    fn batch_order_conflicts_and_write_failures_are_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("batch.db")).unwrap();
        let destination = store.save_collection(None, Some(1), "目标").unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            let mut e = entry();
            e.name = format!("项目{i}");
            e.target = Target::Url {
                url: format!("https://example.com/{i}"),
            };
            ids.push(store.save_entry(&e).unwrap());
        }
        let original = store.entries().unwrap();
        assert!(
            store
                .batch_entries(&[ids[0], 999], Some(destination), false)
                .is_err()
        );
        assert!(store.batch_entries(&[ids[0]], Some(2), true).is_err());
        assert_eq!(store.entries().unwrap(), original);
        store.conn.execute_batch("CREATE TRIGGER fail_batch BEFORE UPDATE ON entries WHEN NEW.position > 3 BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        assert!(
            store
                .batch_entries(&[ids[2], ids[0]], Some(destination), false)
                .is_err()
        );
        assert_eq!(store.entries().unwrap(), original);
        store
            .conn
            .execute_batch("DROP TRIGGER fail_batch;")
            .unwrap();
        store
            .batch_entries(&[ids[2], ids[0]], Some(destination), false)
            .unwrap();
        let moved = store.entries().unwrap();
        assert_eq!(
            moved
                .iter()
                .filter(|e| e.collection_id == destination)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![ids[2], ids[0]]
        );
        store
            .batch_entries(&[ids[1]], Some(destination), true)
            .unwrap();
        assert_eq!(store.entries().unwrap().len(), 4);
        assert!(
            store
                .batch_entries(&[ids[1]], Some(destination), true)
                .is_err()
        );
        store.batch_entries(&[ids[2], ids[0]], None, false).unwrap();
        assert_eq!(store.entries().unwrap().len(), 2);
    }
    #[test]
    fn copy_keeps_source_and_rejects_duplicates_without_partial_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("copy.db")).unwrap();
        let id = store.save_entry(&entry()).unwrap();
        let category = store.save_collection(None, Some(1), "复制目标").unwrap();
        let original = store.entries().unwrap()[0].clone();
        let copy = store.copy_to_collection(id, category).unwrap();
        assert_ne!(copy, id);
        let entries = store.entries().unwrap();
        assert_eq!(entries.iter().find(|e| e.id == id).unwrap(), &original);
        let mut expected = original.clone();
        expected.id = copy;
        expected.collection_id = category;
        expected.position = entries.iter().find(|e| e.id == copy).unwrap().position;
        assert_eq!(entries.iter().find(|e| e.id == copy).unwrap(), &expected);
        for target in [category, original.collection_id, 1, 999] {
            assert!(store.copy_to_collection(id, target).is_err());
            assert_eq!(store.entries().unwrap(), entries);
        }
    }
    #[test]
    fn fresh_database_seeds_group_and_default_category() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(
            store.collections().unwrap(),
            vec![
                Collection {
                    id: 1,
                    parent_id: None,
                    name: "常用".into(),
                    position: 0
                },
                Collection {
                    id: 2,
                    parent_id: Some(1),
                    name: "默认分类".into(),
                    position: 0
                }
            ]
        );
        assert_eq!(store.take_initial().unwrap().entries.len(), 0);
        assert!(store.window_state().unwrap().edge.is_none());
    }
    #[test]
    fn foreign_database_with_tables_is_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreign.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE groups (name TEXT); INSERT INTO groups VALUES ('x');")
            .unwrap();
        let error = open_error(&path);
        assert!(error.contains("无法识别"), "{error}");
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row("SELECT count(*) FROM groups", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn partial_settings_payload_fills_defaults_and_bounds_are_rejected() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut payload = serde_json::to_value(Settings::default()).unwrap();
        for key in ["edge_strip_visible", "edge_strip_width_px", "icon_size"] {
            payload.as_object_mut().unwrap().remove(key);
        }
        store
            .conn
            .execute("UPDATE settings SET payload=?1", [payload.to_string()])
            .unwrap();
        let settings = store.settings().unwrap();
        assert_eq!(settings.edge_strip_width_px, 1);
        assert!(settings.edge_strip_visible);
        assert_eq!(settings.icon_size, 1);
        assert_eq!(settings.hotkey, crate::hotkey::DEFAULT);
        let mut invalid = settings.clone();
        for width in [0, 13] {
            invalid.edge_strip_width_px = width;
            assert!(validate_settings(&invalid).is_err(), "width {width}");
        }
        invalid.edge_strip_width_px = 1;
        invalid.icon_size = 3;
        assert!(validate_settings(&invalid).is_err());
        assert!(store.save_settings(&invalid).is_err());
        invalid.icon_size = 2;
        store.save_settings(&invalid).unwrap();
        assert_eq!(store.settings().unwrap().icon_size, 2);
    }
    #[test]
    fn export_with_another_format_version_is_rejected_and_keeps_data() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.save_entry(&entry()).unwrap();
        for version in [0, 1, 4, 5] {
            let mut data = store.export().unwrap();
            data.format_version = version;
            assert!(store.restore(&data).is_err(), "version {version}");
            assert_eq!(store.entries().unwrap().len(), 1);
            assert_eq!(store.collections().unwrap().len(), 2);
        }
    }
    #[test]
    fn forward_backward_and_last_item_insertions_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("order.db");
        let mut store = Store::open(&path).unwrap();
        let ids: Vec<_> = (0..5)
            .map(|i| {
                let mut e = entry();
                e.target = Target::Url {
                    url: format!("https://example.com/{i}"),
                };
                store.save_entry(&e).unwrap()
            })
            .collect();
        store.move_at(ids[0], ids[1], true).unwrap();
        assert_eq!(
            store
                .entries()
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            [ids[1], ids[0], ids[2], ids[3], ids[4]]
        );
        store.move_at(ids[0], ids[4], true).unwrap();
        store.move_at(ids[3], ids[1], false).unwrap();
        drop(store);
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .entries()
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            [ids[3], ids[1], ids[2], ids[4], ids[0]]
        );
    }
    #[test]
    fn occupied_default_keeps_legacy_items_separate_and_last_category_is_replaced() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let original = store.save_entry(&entry()).unwrap();
        let mut legacy = entry();
        legacy.collection_id = 1;
        let migrated = store.save_entry(&legacy).unwrap();
        store.ensure_categories().unwrap();
        let entries = store.entries().unwrap();
        let original_category = entries
            .iter()
            .find(|e| e.id == original)
            .unwrap()
            .collection_id;
        let migrated_category = entries
            .iter()
            .find(|e| e.id == migrated)
            .unwrap()
            .collection_id;
        assert_ne!(
            original_category, migrated_category,
            "duplicate targets remain in separate categories"
        );
        let group = store.save_collection(None, None, "测试分组").unwrap();
        let default = store
            .collections()
            .unwrap()
            .into_iter()
            .find(|c| c.parent_id == Some(group))
            .unwrap();
        store.delete_collection(default.id).unwrap();
        let replacement = store
            .collections()
            .unwrap()
            .into_iter()
            .find(|c| c.parent_id == Some(group))
            .unwrap();
        store
            .save_collection(Some(replacement.id), Some(1), "移动分类")
            .unwrap();
        assert!(
            store
                .collections()
                .unwrap()
                .iter()
                .any(|c| c.parent_id == Some(group)),
            "moving the last category creates a replacement transactionally"
        );
    }
    #[test]
    fn default_category_creation_is_atomic_and_hotkey_persists() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.conn.execute_batch("CREATE TEMP TRIGGER fail_default BEFORE INSERT ON collections WHEN NEW.parent_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
        let before = store.collections().unwrap();
        assert!(store.save_collection(None, None, "失败分组").is_err());
        assert_eq!(store.collections().unwrap(), before);
        store
            .conn
            .execute_batch("DROP TRIGGER fail_default")
            .unwrap();
        let group = store.save_collection(None, None, "新分组").unwrap();
        assert!(
            store
                .collections()
                .unwrap()
                .iter()
                .any(|c| c.parent_id == Some(group))
        );
        let mut settings = store.settings().unwrap();
        settings.hotkey = "Ctrl+Shift+F12".into();
        store.save_settings(&settings).unwrap();
        assert_eq!(store.settings().unwrap().hotkey, settings.hotkey);
        settings.hotkey = "A".into();
        assert!(store.save_settings(&settings).is_err());
        assert_eq!(store.settings().unwrap().hotkey, "Ctrl+Shift+F12");
    }
    #[test]
    fn migrate_delete_merges_groups_with_colliding_category_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("merge.db")).unwrap();
        let dest = store.save_collection(None, None, "工作").unwrap();
        let tools = store.save_collection(None, Some(1), "工具").unwrap();
        store.save_collection(None, Some(dest), "工具").unwrap();
        let mut e = entry();
        e.collection_id = tools;
        store.save_entry(&e).unwrap();
        store.migrate_delete(1, dest, dir.path()).unwrap();
        let collections = store.collections().unwrap();
        assert!(!collections.iter().any(|c| c.id == 1), "源分组消失");
        let moved = collections.iter().find(|c| c.id == tools).unwrap();
        assert_eq!(
            (moved.parent_id, moved.name.as_str()),
            (Some(dest), "工具 2"),
            "同名分类带着自己的项目一起保留下来"
        );
        assert_eq!(store.entries().unwrap()[0].collection_id, tools);
        assert_eq!(
            collections
                .iter()
                .filter(|c| c.parent_id == Some(dest) && c.name == "默认分类")
                .count(),
            1
        );
        assert_eq!(
            Store::open(&dir.path().join("merge.db"))
                .unwrap()
                .collections()
                .unwrap(),
            collections
        );
    }
    #[test]
    fn migrate_delete_drops_the_empty_placeholder_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("bare.db")).unwrap();
        let dest = store.save_collection(None, None, "工作").unwrap();
        let before = store.collections().unwrap();
        let kept = before.iter().filter(|c| c.parent_id == Some(dest)).count();
        store.migrate_delete(1, dest, dir.path()).unwrap();
        let after = store.collections().unwrap();
        assert!(
            !after.iter().any(|c| c.id == 1),
            "只有默认分类的分组也删得掉"
        );
        assert_eq!(
            after.iter().filter(|c| c.parent_id == Some(dest)).count(),
            kept,
            "空默认分类被丢弃，而不是编号成第二个"
        );
    }
    #[test]
    fn migrate_delete_suffixes_past_reserved_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("reserved.db")).unwrap();
        let dest = store.save_collection(None, None, "工作").unwrap();
        let tools = store.save_collection(None, Some(1), "工具").unwrap();
        for name in ["工具", "工具 2", "工具 3"] {
            store.save_collection(None, Some(dest), name).unwrap();
        }
        let mut e = entry();
        e.collection_id = tools;
        store.save_entry(&e).unwrap();
        store.migrate_delete(1, dest, dir.path()).unwrap();
        let moved = store
            .collections()
            .unwrap()
            .into_iter()
            .find(|c| c.id == tools)
            .expect("source category survives");
        assert_eq!(
            (moved.parent_id, moved.name.as_str()),
            (Some(dest), "工具 4")
        );
    }
    #[test]
    fn migrate_delete_rejects_invalid_targets_without_touching_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("targets.db")).unwrap();
        let dest = store.save_collection(None, None, "工作").unwrap();
        let dest_child = store.save_collection(None, Some(dest), "分类").unwrap();
        let source_child = store.save_collection(None, Some(1), "子分类").unwrap();
        store.save_entry(&entry()).unwrap();
        let before = store.export().unwrap();
        assert!(store.migrate_delete(1, 1, dir.path()).is_err());
        assert!(store.migrate_delete(1, source_child, dir.path()).is_err());
        assert!(store.migrate_delete(1, dest_child, dir.path()).is_err());
        let after = store.export().unwrap();
        assert_eq!(after.collections, before.collections);
        assert_eq!(after.entries, before.entries);
    }
    #[test]
    fn delete_collection_with_contents_removes_the_whole_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("cascade.db")).unwrap();
        let dest = store.save_collection(None, None, "工作").unwrap();
        let tools = store.save_collection(None, Some(1), "工具").unwrap();
        let mut e = entry();
        e.collection_id = tools;
        store.save_entry(&e).unwrap();
        store.save_entry(&entry()).unwrap();
        let kept = store.save_collection(None, Some(dest), "保留分类").unwrap();
        let mut survivor = entry();
        survivor.name = "留下的项目".into();
        survivor.collection_id = kept;
        store.save_entry(&survivor).unwrap();
        store.delete_collection_with_contents(1).unwrap();
        let collections = store.collections().unwrap();
        assert!(!collections.iter().any(|c| c.id == 1 || c.id == tools));
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 1, "只有被删分组里的项目跟着消失");
        assert_eq!(entries[0].collection_id, kept);
    }
    #[test]
    fn delete_collection_with_contents_clears_one_category_and_keeps_its_group() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let tools = store.save_collection(None, Some(1), "工具").unwrap();
        let mut e = entry();
        e.collection_id = tools;
        store.save_entry(&e).unwrap();
        store.delete_collection_with_contents(tools).unwrap();
        let collections = store.collections().unwrap();
        assert!(!collections.iter().any(|c| c.id == tools));
        assert!(
            collections
                .iter()
                .any(|c| c.id == 1 && c.parent_id.is_none())
        );
        assert!(store.entries().unwrap().is_empty());
        assert_eq!(
            collections
                .iter()
                .filter(|c| c.parent_id == Some(1) && c.name == "默认分类")
                .count(),
            1
        );
    }
    #[test]
    fn delete_collection_with_contents_keeps_the_last_group() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.save_entry(&entry()).unwrap();
        let before = store.export().unwrap();
        assert!(store.delete_collection_with_contents(1).is_err());
        let after = store.export().unwrap();
        assert_eq!(after.collections, before.collections);
        assert_eq!(after.entries, before.entries);
    }
    #[test]
    fn conversion_skips_atomic_programs_and_rejects_stale_preview() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("paths.db")).unwrap();
        let mut e = entry();
        e.target = Target::File {
            path: dir.path().join("tool.exe").to_string_lossy().into_owned(),
        };
        store.save_entry(&e).unwrap();
        let preview = crate::commands::conversion_preview(
            &store.entries().unwrap(),
            &[2].into(),
            dir.path(),
            true,
        );
        assert_eq!(preview.entries.len(), 1);
        let mut changed = store.entries().unwrap()[0].clone();
        changed.name = "改名".into();
        store.save_entry(&changed).unwrap();
        assert!(store.convert_entries(&preview).is_err());
        let current = crate::commands::conversion_preview(
            &store.entries().unwrap(),
            &[2].into(),
            dir.path(),
            true,
        );
        store.convert_entries(&current).unwrap();
        assert_eq!(
            crate::paths::absolute(store.entries().unwrap()[0].target.value(), dir.path()),
            dir.path().join("tool.exe")
        );
        let mut e = entry();
        e.target = Target::Program {
            path: "C:\\app.exe".into(),
            arguments: String::new(),
            working_dir: "D:\\work".into(),
            elevated: false,
            show: ShowMode::Maximized,
        };
        let p = crate::commands::conversion_preview(
            &[e.clone()],
            &[2].into(),
            Path::new("C:\\base"),
            true,
        );
        assert!(p.entries.is_empty());
        assert_eq!(p.selected, [e]);
        assert_eq!(p.skipped.len(), 1);
    }
    #[test]
    fn relative_target_duplicates_and_embedded_icons_survive_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icons.db");
        let mut store = Store::open(&path).unwrap();
        let mut item = entry();
        item.target = Target::File {
            path: dir.path().join("test.txt").to_string_lossy().into_owned(),
        };
        item.icon = crate::icons::encode(image::RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([255, 0, 0, 255]),
        ))
        .unwrap();
        store.save_entry(&item).unwrap();
        let mut relative = item.clone();
        relative.target = Target::File {
            path: ".\\test.txt".into(),
        };
        assert!(store.save_entry(&relative).is_err());
        let exported = serde_json::to_vec(&store.export().unwrap()).unwrap();
        drop(store);
        let mut reopened = Store::open(&path).unwrap();
        reopened
            .restore(&serde_json::from_slice(&exported).unwrap())
            .unwrap();
        assert_eq!(reopened.entries().unwrap()[0].icon, item.icon);
        assert_eq!(image::load_from_memory(&item.icon).unwrap().width(), 32);
    }
    #[test]
    fn persists_and_rejects_duplicates_without_losing_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut store = Store::open(&path).unwrap();
        let id = store.save_entry(&entry()).unwrap();
        assert!(store.save_entry(&entry()).is_err());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.entries().unwrap()[0].id, id);
        assert_eq!(store.entries().unwrap().len(), 1);
    }
    #[test]
    fn batch_skips_duplicates_and_rolls_back_on_write_failure() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let first = entry();
        let mut second = entry();
        second.target = Target::Url {
            url: "https://example.org".into(),
        };
        assert_eq!(
            store.add_entries(&[first.clone(), first.clone()]).unwrap(),
            (1, 1)
        );
        let original = store.entries().unwrap();
        let mut invalid = second.clone();
        invalid.collection_id = 999;
        assert!(store.add_entries(&[second.clone(), invalid]).is_err());
        assert_eq!(store.entries().unwrap(), original);
        assert_eq!(store.add_entries(&[first, second]).unwrap(), (1, 1));
    }
    #[test]
    fn drag_order_is_persistent_and_does_not_touch_other_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sort.db");
        let mut store = Store::open(&path).unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            let mut e = entry();
            e.target = Target::Url {
                url: format!("https://example.com/{i}"),
            };
            ids.push(store.save_entry(&e).unwrap());
        }
        let group = store.save_collection(None, None, "其他").unwrap();
        let category = store.save_collection(None, Some(group), "分类").unwrap();
        let mut other = entry();
        other.collection_id = category;
        let other_id = store.save_entry(&other).unwrap();
        let original_other = store
            .entries()
            .unwrap()
            .into_iter()
            .find(|e| e.id == other_id)
            .unwrap();
        store.move_at(ids[2], ids[0], false).unwrap();
        assert!(store.move_at(ids[0], other_id, false).is_err());
        drop(store);
        let entries = Store::open(&path).unwrap().entries().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.collection_id == 2)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            [ids[2], ids[0], ids[1]]
        );
        assert_eq!(
            entries.iter().find(|e| e.id == other_id).unwrap(),
            &original_other
        );
    }
    #[test]
    fn validates_two_levels_and_nonempty_deletion() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let child = store.save_collection(None, Some(1), "分类").unwrap();
        assert!(store.save_collection(None, Some(child), "第三层").is_err());
        assert!(store.delete_collection(1).is_err());
        store.save_entry(&entry()).unwrap();
        assert!(store.delete_collection(1).is_err());
    }
    #[test]
    fn invalid_restore_is_atomic_and_valid_roundtrip_preserves_fields() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.save_entry(&entry()).unwrap();
        let original = store.export().unwrap();
        let mut invalid = original.clone();
        invalid.entries[0].collection_id = 999;
        assert!(store.restore(&invalid).is_err());
        assert_eq!(store.entries().unwrap(), original.entries);
        store.delete_entry(original.entries[0].id).unwrap();
        store.restore(&original).unwrap();
        assert_eq!(store.entries().unwrap(), original.entries);
    }
    #[test]
    fn backup_includes_wal_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("source.db")).unwrap();
        store.save_entry(&entry()).unwrap();
        let backup = dir.path().join("backup.db");
        store.backup(&backup).unwrap();
        assert_eq!(
            Store::open(&backup).unwrap().entries().unwrap(),
            store.entries().unwrap()
        );
        assert!(store.backup(&backup).is_err());
    }
    #[test]
    fn other_schema_versions_are_rejected_without_downgrading() {
        // Diascia restarts its version counter, so a pre-reset database and a future one
        // share one wording; both must be refused and left byte-identical in version.
        // Version 1 is the real previous schema, whose `entries` had no icon/dup_key columns.
        for version in [1, 5, 99] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("other.db");
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE collections (id INTEGER PRIMARY KEY, parent_id INTEGER, name TEXT NOT NULL, position INTEGER NOT NULL);
                CREATE TABLE entries (id INTEGER PRIMARY KEY, collection_id INTEGER NOT NULL, payload TEXT NOT NULL, position INTEGER NOT NULL);
                CREATE TABLE settings (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);
                CREATE TABLE local_state (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);
                INSERT INTO settings VALUES (1, '{}');").unwrap();
            conn.pragma_update(None, "user_version", version).unwrap();
            drop(conn);
            let error = open_error(&path);
            assert!(
                error.contains(&format!("user_version={version}")),
                "{error}"
            );
            assert_eq!(
                Connection::open(&path)
                    .unwrap()
                    .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                version
            );
        }
    }
    #[test]
    fn failure_mid_restore_rolls_back_deletions_and_partial_inserts() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        store.save_entry(&entry()).unwrap();
        let original = store.export().unwrap();
        let mut replacement = original.clone();
        replacement.entries[0].name = "不应留下的修改".into();
        let mut second = entry();
        second.id = 99;
        second.target = Target::Url {
            url: "https://example.org".into(),
        };
        replacement.entries.push(second);
        store.conn.execute_batch("CREATE TEMP TRIGGER fail_import BEFORE INSERT ON entries WHEN NEW.id=99 BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;").unwrap();
        assert!(store.restore(&replacement).is_err());
        assert_eq!(store.entries().unwrap(), original.entries);
        assert_eq!(store.collections().unwrap(), original.collections);
    }
    #[test]
    fn unknown_fields_are_rejected_instead_of_lost_on_restore() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let mut data = serde_json::to_value(store.export().unwrap()).unwrap();
        data["settings"]["future_setting"] = true.into();
        assert!(serde_json::from_value::<Export>(data).is_err());
    }
    #[test]
    fn moving_an_entry_keeps_order_and_does_not_modify_other_groups() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let first = store.save_entry(&entry()).unwrap();
        let mut second = entry();
        second.name = "另一个".into();
        second.target = Target::Url {
            url: "https://example.org".into(),
        };
        let second_id = store.save_entry(&second).unwrap();
        store.move_entry(second_id, -1).unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(
            entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second_id, first]
        );
        let group = store.save_collection(None, None, "工作").unwrap();
        let mut moved = entries[0].clone();
        moved.collection_id = group;
        store.save_entry(&moved).unwrap();
        assert_eq!(
            store
                .entries()
                .unwrap()
                .iter()
                .filter(|e| e.collection_id == 2)
                .count(),
            1
        );
    }
}
