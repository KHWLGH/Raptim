use crate::model::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

pub struct Store {
    conn: Connection,
    base: std::path::PathBuf,
    initial: Option<Export>,
    dirty_icons: HashSet<i64>,
    all_icons_dirty: bool,
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
                // Raptim restarts its version counter, so a foreign version number cannot tell a
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
            dirty_icons: HashSet::new(),
            all_icons_dirty: false,
        };
        // Fail visibly on damaged payload/settings. Never silently replace user data.
        let validated = store.export()?;
        validate_export(&validated)?;
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
        let all = self.entries_without_icons()?;
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
            // Choosing the category the batch already sits in is not an operation. Refuse it by name:
            // until now the duplicate scan below happened to reject it too, but it reported a clash
            // with a project that was not a clash, which sent the reader looking for one.
            if selected.iter().any(|e| e.collection_id == dest) {
                return Err(Error::Invalid("项目已经在所选分类里；整批未修改".into()));
            }
            let mut taken = self.slots_of(&[])?;
            let keys: Vec<String> = selected.iter().map(|e| self.dup_key(e)).collect();
            if let Some(name) = Self::first_clash(
                &mut taken,
                selected
                    .iter()
                    .zip(&keys)
                    .map(|(e, key)| (dest, *e, key.as_str())),
            ) {
                return Err(Error::Invalid(format!(
                    "目标分类存在重复项目：{name}；整批未修改"
                )));
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
                        "INSERT INTO entries(collection_id,payload,position,icon,dup_key) SELECT ?1,?2,?3,icon,?4 FROM entries WHERE id=?5",
                        params![dest, payload, position, dup_key, entry.id],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE entries SET collection_id=?1,payload=?2,position=?3,dup_key=?4 WHERE id=?5",
                        params![dest, payload, position, dup_key, entry.id],
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
    /// The dedup identity this store commits: the target as the Shell would see it, resolved
    /// against the directory the database lives in. Every duplicate check and the `dup_key` column
    /// go through here, so a probe cannot answer about a different string than the one it indexes.
    fn storage_key(&self, target: &Target) -> String {
        crate::paths::resolve_for_shell(target, &self.base).duplicate_key()
    }
    /// The `payload` column: the entry without its icon, which lives in the `icon` BLOB column so
    /// that reading a library never decodes icon bytes. A version 1 database kept both in one JSON
    /// document, where `Vec<u8>` costs ~3.6 text characters per byte and one token each.
    fn payload_of(entry: &Entry) -> Result<String> {
        let mut row = entry.clone();
        row.icon = Vec::new();
        Ok(serde_json::to_string(&row)?)
    }
    /// The dedup identity, materialised because `storage_key` absolutises paths against the data
    /// directory and lower-cases URLs, neither of which SQLite can express. It exists so the
    /// duplicate-target check is an index probe instead of a full-table re-read.
    fn dup_key(&self, entry: &Entry) -> String {
        self.storage_key(&entry.target)
    }
    /// The slots the committed table already holds, as `(collection_id, dup_key)`, minus the rows in
    /// `except`: a rewrite must not be refused by the slot it is itself vacating. Only the dedup
    /// columns are read — a batch check must not re-parse every payload.
    fn slots_of(&self, except: &[i64]) -> Result<HashSet<(i64, String)>> {
        let skip: HashSet<i64> = except.iter().copied().collect();
        let mut statement = self
            .conn
            .prepare("SELECT collection_id,dup_key,id FROM entries")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = HashSet::new();
        for row in rows {
            let (collection_id, dup_key, id) = row?;
            if !skip.contains(&id) {
                out.insert((collection_id, dup_key));
            }
        }
        Ok(out)
    }
    /// The name of the first row whose `(collection, key)` slot is already taken, counting the
    /// earlier rows of the same batch. Keys come from `dup_key`, so a probe never answers about a
    /// different string than the one the write goes on to index. The collection is a parameter
    /// rather than read off the entry because a batch move asks about the *destination*. Each write
    /// path hands in the `taken` that is correct for it — the committed table for a rewrite, the
    /// whole library for a move, nothing for a replacement — and picks its own outcome:
    /// `add_entries` skips a repeat silently, every other path refuses the batch.
    fn first_clash<'a, 'b>(
        taken: &mut HashSet<(i64, String)>,
        batch: impl Iterator<Item = (i64, &'a Entry, &'b str)>,
    ) -> Option<String> {
        for (collection_id, entry, key) in batch {
            if !taken.insert((collection_id, key.to_owned())) {
                return Some(entry.name.clone());
            }
        }
        None
    }
    /// A whole-library replacement, checked against the identity that will actually be indexed.
    /// Two rows that only *spell* the same target differently pass `validate_export`, which compares
    /// the raw value on purpose so an export stays legal whatever directory the database sits in,
    /// and then land on one `dup_key` — after which every edit of either is refused. Only the import
    /// path asks this: `restore` itself stays lax, so sorting and reordering keep working on a
    /// library that already holds a pair.
    pub fn export_clash(&self, entries: &[Entry]) -> Option<String> {
        let keys: Vec<String> = entries.iter().map(|e| self.dup_key(e)).collect();
        let mut taken = HashSet::new();
        Self::first_clash(
            &mut taken,
            entries
                .iter()
                .zip(&keys)
                .map(|(e, key)| (e.collection_id, e, key.as_str())),
        )
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
        rows.map(|row| {
            let (id, collection_id, payload, position, icon) = row?;
            let mut e: Entry = serde_json::from_str(&payload)?;
            e.id = id;
            e.collection_id = collection_id;
            e.position = position;
            e.icon = icon.unwrap_or_default();
            Ok(e)
        })
        .collect()
    }
    fn entries_without_icons(&self) -> Result<Vec<Entry>> {
        let mut statement = self.conn.prepare(
            "SELECT id,collection_id,payload,position FROM entries ORDER BY position,id",
        )?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, collection_id, payload, position) = row?;
            let mut e: Entry = serde_json::from_str(&payload)?;
            e.id = id;
            e.collection_id = collection_id;
            e.position = position;
            e.icon = Vec::new();
            Ok(e)
        })
        .collect()
    }
    fn entry_positions(&self) -> Result<Vec<(i64, i64)>> {
        let mut statement = self
            .conn
            .prepare("SELECT id,collection_id FROM entries ORDER BY position,id")?;
        let rows = statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    /// Reads all entry metadata while reusing in-memory icon bytes for rows whose icons have not
    /// changed since the last refresh. Only newly inserted rows or rows whose icon column was
    /// modified fetch their BLOB from SQLite.
    pub fn entries_reusing_icons(
        &mut self,
        previous: &[Entry],
    ) -> Result<(Vec<Entry>, HashSet<i64>)> {
        if self.all_icons_dirty || previous.is_empty() {
            self.all_icons_dirty = false;
            self.dirty_icons.clear();
            let all = self.entries()?;
            let rehashed = all.iter().map(|e| e.id).collect();
            return Ok((all, rehashed));
        }
        let dirty = std::mem::take(&mut self.dirty_icons);
        let mut known_icons: HashMap<i64, Vec<u8>> = previous
            .iter()
            .filter(|e| !dirty.contains(&e.id))
            .map(|e| (e.id, e.icon.clone()))
            .collect();
        let mut list = self.entries_without_icons()?;
        let mut icon_stmt = self.conn.prepare("SELECT icon FROM entries WHERE id=?1")?;
        let mut rehashed = HashSet::new();
        for entry in &mut list {
            if let Some(icon) = known_icons.remove(&entry.id) {
                entry.icon = icon;
            } else {
                let icon: Option<Vec<u8>> = icon_stmt
                    .query_row([entry.id], |r| r.get(0))
                    .optional()?
                    .flatten();
                entry.icon = icon.unwrap_or_default();
                rehashed.insert(entry.id);
            }
        }
        Ok((list, rehashed))
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
        // Rewriting a target rewrites its slot, so this is the one place a rewrite can create a pair
        // of rows that index identically. 重新定位 and 路径转换 both land here, and the collision they
        // write is what makes both rows uneditable afterwards and `Store::open` refuse to boot.
        let rewritten: Vec<i64> = updates.iter().map(|e| e.id).collect();
        let mut taken = self.slots_of(&rewritten)?;
        if let Some(name) = Self::first_clash(
            &mut taken,
            updates
                .iter()
                .zip(&keys)
                .map(|(e, key)| (e.collection_id, e, key.as_str())),
        ) {
            return Err(Error::Invalid(format!(
                "目标分类已有重复项目：{name}；未修改任何项目"
            )));
        }
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
        for (e, old) in updates.iter().zip(originals) {
            if e.icon != old.icon {
                self.dirty_icons.insert(e.id);
            }
        }
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
        let has_entries: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE collection_id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if groups.iter().any(|c| c.parent_id == Some(id)) || has_entries {
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
            self.dirty_icons.insert(entry.id);
            entry.id
        };
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
        self.dirty_icons.insert(id);
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
        // Only the dedup columns are read. The previous shape re-parsed every payload, icon bytes
        // included, to answer "is this target already here".
        let mut keys = self.slots_of(&[])?;
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
        let all = self.entry_positions()?;
        let &(_, source_collection) = all
            .iter()
            .find(|(eid, _)| *eid == id)
            .ok_or_else(|| Error::Invalid("项目已不存在".into()))?;
        if !all
            .iter()
            .any(|(eid, cid)| *eid == before && *cid == source_collection)
        {
            return Err(Error::Invalid("只能在当前分类内排序".into()));
        }
        if id == before {
            return Ok(());
        }
        let mut order: Vec<_> = all
            .into_iter()
            .filter(|(eid, cid)| *cid == source_collection && *eid != id)
            .map(|(eid, _)| eid)
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
        let all = self.entry_positions()?;
        let Some(&(_, collection_id)) = all.iter().find(|(eid, _)| *eid == id) else {
            return Err(Error::Invalid("项目已不存在".into()));
        };
        let mut siblings: Vec<_> = all
            .into_iter()
            .filter(|(_, cid)| *cid == collection_id)
            .map(|(eid, _)| eid)
            .collect();
        let index = siblings.iter().position(|eid| *eid == id).unwrap();
        let next = index as i64 + i64::from(direction.signum());
        if next < 0 || next as usize >= siblings.len() {
            return Ok(());
        }
        siblings.swap(index, next as usize);
        let tx = self.conn.transaction()?;
        for (position, eid) in siblings.iter().enumerate() {
            tx.execute(
                "UPDATE entries SET position=?1 WHERE id=?2",
                params![position as i64, eid],
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
        self.all_icons_dirty = true;
        self.dirty_icons.clear();
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
    pub fn reorder_collection_at(&mut self, id: i64, before: i64, after: bool) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let collections = self.collections()?;
        let source = collections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Invalid("分组不存在".into()))?;
        let parent = source.parent_id;
        if !collections
            .iter()
            .any(|c| c.id == before && c.parent_id == parent)
        {
            return Err(Error::Invalid("只能在同一级排序".into()));
        }
        if id == before {
            return Ok(());
        }
        let mut ids: Vec<_> = collections
            .iter()
            .filter(|c| c.parent_id == parent && c.id != id)
            .map(|c| c.id)
            .collect();
        let at = ids.iter().position(|c| *c == before).unwrap();
        ids.insert(at + usize::from(after), id);
        for (position, cid) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE collections SET position=?1 WHERE id=?2",
                params![position as i64, cid],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn move_to_collection(&mut self, id: i64, destination: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let Some((current_collection, payload, dup_key)) = tx
            .query_row(
                "SELECT collection_id,payload,dup_key FROM entries WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Err(Error::Invalid("项目不存在".into()));
        };
        if current_collection == destination {
            return Ok(());
        }
        if !self
            .collections()?
            .iter()
            .any(|c| c.id == destination && c.parent_id.is_some())
        {
            return Err(Error::Invalid("目标分类已不存在".into()));
        }
        let mut entry: Entry = serde_json::from_str(&payload)?;
        let clash: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE collection_id=?1 AND dup_key=?2 AND id<>?3)",
            params![destination, dup_key, id],
            |r| r.get(0),
        )?;
        if clash {
            return Err(Error::Invalid(format!(
                "目标分类已有重复项目：{}",
                entry.name
            )));
        }
        let end: i64 = tx.query_row(
            "SELECT coalesce(max(position),-1)+1 FROM entries WHERE collection_id=?1",
            [destination],
            |r| r.get(0),
        )?;
        entry.id = id;
        entry.collection_id = destination;
        entry.position = end;
        let next_payload = Self::payload_of(&entry)?;
        tx.execute(
            "UPDATE entries SET collection_id=?1,payload=?2,position=?3 WHERE id=?4",
            params![destination, next_payload, end, id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn migrate_delete(&mut self, id: i64, destination: i64) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let collections = self.collections()?;
        let source = collections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Invalid("分组不存在".into()))?;
        let dest = collections
            .iter()
            .find(|c| c.id == destination)
            .ok_or_else(|| Error::Invalid("请选择迁移目标".into()))?;
        if id == destination
            || dest.parent_id == Some(id)
            || (source.parent_id.is_none() && dest.parent_id.is_some())
        {
            return Err(Error::Invalid("迁移目标无效".into()));
        }
        let mut stmt = tx.prepare(
            "SELECT id,payload,dup_key FROM entries WHERE collection_id=?1 ORDER BY position,id",
        )?;
        let moving_entries: Vec<(i64, String, String)> = stmt
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        drop(stmt);

        if !moving_entries.is_empty() {
            let mut stmt = tx.prepare("SELECT dup_key FROM entries WHERE collection_id=?1")?;
            let mut taken: HashSet<String> = stmt
                .query_map([destination], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            drop(stmt);
            let end: i64 = tx.query_row(
                "SELECT coalesce(max(position),-1)+1 FROM entries WHERE collection_id=?1",
                [destination],
                |r| r.get(0),
            )?;
            for (offset, (entry_id, payload, dup_key)) in moving_entries.iter().enumerate() {
                let mut entry: Entry = serde_json::from_str(payload)?;
                if !taken.insert(dup_key.clone()) {
                    return Err(Error::Invalid(format!(
                        "目标分类已有重复项目：{}",
                        entry.name
                    )));
                }
                let position = end + offset as i64;
                entry.id = *entry_id;
                entry.collection_id = destination;
                entry.position = position;
                tx.execute(
                    "UPDATE entries SET collection_id=?1,payload=?2,position=?3 WHERE id=?4",
                    params![destination, Self::payload_of(&entry)?, position, entry_id],
                )?;
            }
        }

        let next = collections
            .iter()
            .filter(|c| c.parent_id == Some(destination))
            .map(|c| c.position)
            .max()
            .unwrap_or(-1)
            + 1;
        // Track names claimed under `destination` in memory so sibling renames within this same
        // migration avoid colliding with one another before the transaction commits.
        let mut taken: HashSet<String> = collections
            .iter()
            .filter(|c| c.parent_id == Some(destination))
            .map(|c| c.name.trim().to_lowercase())
            .collect();
        let mut dropped = Vec::new();
        let mut moved: i64 = 0;
        for c in collections.iter().filter(|c| c.parent_id == Some(id)) {
            let has_entries: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM entries WHERE collection_id=?1)",
                [c.id],
                |r| r.get(0),
            )?;
            // 默认分类 is a placeholder this app creates by itself, so an empty one is not
            // something to relocate and keep: renaming it would strand a `默认分类 2` row that
            // normalize_categories never reuses.
            if c.name == "默认分类" && taken.contains("默认分类") && !has_entries {
                dropped.push(c.id);
                continue;
            }
            let stem = &c.name;
            let mut name = stem.clone();
            let mut suffix = 2;
            while taken.contains(&name.trim().to_lowercase()) {
                name = format!("{stem} {suffix}");
                suffix += 1;
            }
            taken.insert(name.trim().to_lowercase());
            tx.execute(
                "UPDATE collections SET name=?1,parent_id=?2,position=?3 WHERE id=?4",
                params![name, destination, next + moved, c.id],
            )?;
            moved += 1;
        }
        for dropped_id in dropped {
            tx.execute("DELETE FROM collections WHERE id=?1", [dropped_id])?;
        }
        tx.execute("DELETE FROM collections WHERE id=?1", [id])?;
        normalize_categories(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn sort_names(&mut self, selected: &HashSet<i64>) -> Result<()> {
        if selected.is_empty() {
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut stmt =
            tx.prepare("SELECT id,collection_id,payload FROM entries ORDER BY position,id")?;
        let mut entries: Vec<(i64, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .filter_map(|row| match row {
                Ok((id, collection_id, payload)) if selected.contains(&collection_id) => Some(
                    serde_json::from_str::<Entry>(&payload)
                        .map(|e| (id, e.name))
                        .map_err(crate::model::Error::from),
                ),
                Ok(_) => None,
                Err(e) => Some(Err(e.into())),
            })
            .collect::<std::result::Result<Vec<_>, crate::model::Error>>()?;
        drop(stmt);
        entries.sort_by(|(aid, aname), (bid, bname)| {
            (aname.to_lowercase(), *aid).cmp(&(bname.to_lowercase(), *bid))
        });
        for (position, (id, _)) in entries.into_iter().enumerate() {
            tx.execute(
                "UPDATE entries SET position=?1 WHERE id=?2",
                params![position as i64, id],
            )?;
        }
        tx.commit()?;
        Ok(())
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
    if settings.theme_mode > 2 {
        return Err(Error::Invalid("外观主题必须为跟随系统、浅色或深色".into()));
    }
    if settings.icon_size > 2 {
        return Err(Error::Invalid("图标大小必须为小、中或大".into()));
    }
    crate::hotkey::parse(&settings.hotkey).map_err(Error::Invalid)?;
    if !(1..=12).contains(&settings.edge_strip_width_px) {
        return Err(Error::Invalid("露边宽度应为 1–12 个物理像素".into()));
    }
    if !(100..=2000).contains(&settings.edge_collapse_delay_ms) {
        return Err(Error::Invalid("贴边收起延迟应在 100–2000 ms 之间".into()));
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
            "此导出文件为版本 {}，Raptim 只能恢复版本 {SCHEMA_VERSION} 的导出",
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

    #[test]
    fn entries_crud_dedup_and_ordering() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let first = store.save_entry(&entry()).unwrap();
        assert!(store.save_entry(&entry()).is_err());

        let mut second = entry();
        second.name = "Alpha".into();
        second.icon = vec![9u8; 32];
        second.target = Target::Url {
            url: "https://alpha.example.com".into(),
        };
        let second_id = store.save_entry(&second).unwrap();

        store.save_icon(first, &[7u8; 64]).unwrap();
        store.move_at(second_id, first, false).unwrap();
        let (entries, rehashed) = store.entries_reusing_icons(&[]).unwrap();
        assert_eq!(rehashed.len(), 2);
        assert_eq!(
            entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second_id, first]
        );
        assert_eq!(
            entries.iter().find(|e| e.id == first).unwrap().icon,
            vec![7u8; 64]
        );

        // Reordering does not dirty any icon, so incremental refresh reuses in-memory bytes.
        store.move_entry(second_id, 1).unwrap();
        let (entries_after_move, rehashed_after_move) =
            store.entries_reusing_icons(&entries).unwrap();
        assert!(rehashed_after_move.is_empty());
        assert_eq!(
            entries_after_move.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![first, second_id]
        );

        let other = store.save_collection(None, Some(1), "另一分类").unwrap();
        let copied = store.copy_to_collection(first, other).unwrap();
        assert_ne!(copied, first);
        store
            .batch_entries(&[second_id], Some(other), false)
            .unwrap();
        let (after_batch, rehashed_batch) =
            store.entries_reusing_icons(&entries_after_move).unwrap();
        assert_eq!(rehashed_batch, HashSet::from([copied]));
        assert_eq!(
            after_batch.iter().find(|e| e.id == second_id).unwrap().icon,
            vec![9u8; 32]
        );
        store.delete_entry(first).unwrap();
        assert_eq!(store.entries().unwrap().len(), 2);
    }

    #[test]
    fn collections_hierarchy_and_deletion() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(store.collections().unwrap().len(), 2);

        let group = store.save_collection(None, None, "工作").unwrap();
        let tools = store.save_collection(None, Some(1), "工具").unwrap();
        let mut e = entry();
        e.collection_id = tools;
        store.save_entry(&e).unwrap();

        store.migrate_delete(1, group).unwrap();
        let collections = store.collections().unwrap();
        assert!(!collections.iter().any(|c| c.id == 1));
        assert_eq!(store.entries().unwrap()[0].collection_id, tools);

        store.delete_collection_with_contents(tools).unwrap();
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn export_restore_and_settings_roundtrip() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut item = entry();
        item.icon = vec![1, 2, 3, 4];
        store.save_entry(&item).unwrap();

        let mut settings = store.settings().unwrap();
        settings.hotkey = "Ctrl+Shift+F12".into();
        store.save_settings(&settings).unwrap();

        let state = crate::session::WindowState {
            always_on_top: true,
            width: 440.,
            height: 640.,
            group: 1,
            category: 2,
            ..Default::default()
        };
        store.save_window_state(&state).unwrap();

        let exported = serde_json::to_vec(&store.export().unwrap()).unwrap();
        store.delete_entry(1).unwrap();
        store
            .restore(&serde_json::from_slice(&exported).unwrap())
            .unwrap();
        assert_eq!(store.entries().unwrap()[0].icon, vec![1, 2, 3, 4]);
        assert_eq!(store.settings().unwrap().hotkey, "Ctrl+Shift+F12");
        assert_eq!(store.window_state().unwrap(), state);
    }
}
