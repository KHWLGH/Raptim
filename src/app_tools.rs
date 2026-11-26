// Installed software discovery, broken entry inspection, and target relocation workflows.
use super::*;

#[derive(Clone)]
pub(super) struct BrokenEntry {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) target: String,
    pub(super) reason: String,
    pub(super) relocatable: bool,
}

impl App {
    /// Arms the in-window relocate panel. Returns false when the target has no path to
    /// re-point, so callers can fall back to a plain status message.
    pub(super) fn show_relocate(
        &mut self,
        id: i64,
        message: &str,
        error_relocatable: bool,
    ) -> bool {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return false;
        };
        if !error_relocatable || !entry.target.relocatable() {
            return false;
        }
        self.relocate_pending = id;
        self.main.set_relocate_name(entry.name.clone().into());
        self.main.set_relocate_target(entry.target.value().into());
        self.main.set_relocate_message(message.into());
        self.main.set_relocate_open(true);
        true
    }

    pub(super) fn begin_relocate(&mut self) {
        if self.relocate_pending == 0 {
            return;
        }
        self.dialog(DialogKind::RelocateTarget);
    }

    pub(super) fn apply_relocate(&mut self, value: String) -> Result<()> {
        let id = self.relocate_pending;
        let Some(original) = self.entries.iter().find(|e| e.id == id).cloned() else {
            self.relocate_pending = 0;
            return Ok(());
        };
        let mut updated = original.clone();
        updated.target = original.target.with_path(&value)?;
        // The embedded icon was extracted from the target this entry used to point at.
        updated.icon = Vec::new();
        self.store
            .update_targets_checked(std::slice::from_ref(&updated), &[original])?;
        self.relocate_pending = 0;
        self.main.set_relocate_open(false);
        self.broken_items.retain(|item| item.id != id);
        if self.main.get_broken_open() {
            self.sync_broken_ui();
        }
        self.report_refresh();
        self.fill_icons(vec![(id, updated.target)]);
        Ok(())
    }

    pub(super) fn begin_discover_apps(&mut self) {
        if self.main.get_busy() {
            return;
        }
        if !self.finish_inline() {
            return;
        }
        self.user_summon_edge();
        self.discovered_apps.clear();
        self.discovered_checked.clear();
        self.main
            .set_discover_target_name(self.collection_name(self.collection));
        self.main.set_discover_filter("".into());
        self.main.set_discover_items(ModelRc::default());
        self.main.set_discover_selected_count(0);
        self.main.set_discover_loading(true);
        self.main.set_discover_open(true);
        self.main.invoke_focus_discover_filter();

        let existing: std::collections::HashSet<String> = self
            .entries
            .iter()
            .filter(|e| e.collection_id == self.collection)
            .map(|e| raptim::paths::resolve_for_shell(&e.target, &self.data_dir).duplicate_key())
            .collect();
        let data_dir = self.data_dir.clone();
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let mut apps = platform::discover_installed_entries();
            apps.retain(|app| {
                let key = raptim::paths::resolve_for_shell(&app.target, &data_dir).duplicate_key();
                !existing.contains(&key)
            });
            sink.send(AppEvent::DiscoveredAppsReady(apps));
        });
    }

    pub(super) fn render_discover_list(&mut self) {
        let needle = self.main.get_discover_filter().trim().to_lowercase();
        let mut options = Vec::new();
        let selected_count = self.discovered_checked.iter().filter(|&&c| c).count() as i32;
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if !needle.is_empty()
                && !app.name.to_lowercase().contains(&needle)
                && !app.detail.to_lowercase().contains(&needle)
            {
                continue;
            }
            let checked = self.discovered_checked.get(index).copied().unwrap_or(false);
            options.push(DiscoverOption {
                checked,
                index: index as i32,
                name: app.name.clone().into(),
                detail: app.detail.clone().into(),
                kind: app.target.kind_index(),
            });
        }
        self.main.set_discover_selected_count(selected_count);
        self.main
            .set_discover_items(Rc::new(VecModel::from(options)).into());
    }

    pub(super) fn toggle_discovered(&mut self, idx: i32) {
        let Ok(index) = usize::try_from(idx) else {
            return;
        };
        if let Some(flag) = self.discovered_checked.get_mut(index) {
            *flag = !*flag;
            self.render_discover_list();
        }
    }

    pub(super) fn select_all_discovered(&mut self, checked: bool) {
        let needle = self.main.get_discover_filter().trim().to_lowercase();
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if (needle.is_empty()
                || app.name.to_lowercase().contains(&needle)
                || app.detail.to_lowercase().contains(&needle))
                && let Some(flag) = self.discovered_checked.get_mut(index)
            {
                *flag = checked;
            }
        }
        self.render_discover_list();
    }

    pub(super) fn import_discovered(&mut self) {
        if self.discovered_apps.is_empty() {
            return;
        }
        let mut new_entries = Vec::new();
        let mut icon_sources = std::collections::HashMap::new();
        for (index, app) in self.discovered_apps.iter().enumerate() {
            if !self.discovered_checked.get(index).copied().unwrap_or(false) {
                continue;
            }
            let mut target = app.target.clone();
            if self.settings.relative_paths
                && let Ok(rel) = raptim::paths::convert_target(&target, &self.data_dir, true)
            {
                target = rel;
            }
            if let Some(source) = &app.icon_source {
                let key = raptim::paths::resolve_for_shell(&target, &self.data_dir).duplicate_key();
                icon_sources.insert(key, source.clone());
            }
            new_entries.push(Entry {
                id: 0,
                collection_id: self.collection,
                name: app.name.clone(),
                aliases: String::new(),
                notes: String::new(),
                target,
                position: 0,
                icon: Vec::new(),
            });
        }
        if new_entries.is_empty() {
            self.status("请先勾选要导入的软件");
            return;
        }
        match self.store.add_entries(&new_entries) {
            Ok((added, skipped)) => {
                self.main.set_discover_open(false);
                self.main.invoke_focus_browser();
                self.report_refresh();
                let wanted: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|e| {
                        e.collection_id == self.collection
                            && e.icon.is_empty()
                            && !matches!(e.target, Target::Url { .. })
                    })
                    .map(|e| {
                        let key = raptim::paths::resolve_for_shell(&e.target, &self.data_dir)
                            .duplicate_key();
                        (e.id, e.target.clone(), icon_sources.get(&key).cloned())
                    })
                    .collect();
                self.fill_icons_with_sources(wanted);
                self.status(if skipped > 0 {
                    format!("已导入 {added} 项，跳过 {skipped} 项重复")
                } else {
                    format!("已导入 {added} 项软件")
                });
            }
            Err(e) => self.status(e),
        }
    }

    pub(super) fn begin_broken_check(&mut self) {
        if !self.finish_inline() {
            return;
        }
        self.user_summon_edge();
        self.broken_items.clear();
        self.main.set_broken_items(ModelRc::default());
        self.main.set_broken_loading(true);
        self.main.set_broken_open(true);
        let entries = self.entries.clone();
        let data_dir = self.data_dir.clone();
        let epoch = self.data_epoch;
        let sink = self.events.clone();
        std::thread::spawn(move || {
            let needs_apps = entries
                .iter()
                .any(|e| matches!(e.target, Target::App { .. }));
            let installed: std::collections::HashSet<String> = if needs_apps {
                platform::installed_apps()
                    .into_iter()
                    .map(|(aumid, _)| aumid.to_lowercase())
                    .collect()
            } else {
                Default::default()
            };
            let mut found = Vec::new();
            for entry in entries {
                let resolved = raptim::paths::resolve_for_shell(&entry.target, &data_dir);
                let reason = match &resolved {
                    Target::Url { .. } => None,
                    Target::App { aumid } => {
                        if installed.contains(&aumid.to_lowercase()) {
                            None
                        } else {
                            Some("未找到对应的已安装应用")
                        }
                    }
                    Target::Folder { path } => {
                        let p = std::path::Path::new(path);
                        let low = path.to_ascii_lowercase();
                        if low.starts_with("shell:") || low.starts_with("::{") || p.is_dir() {
                            None
                        } else {
                            Some("目标文件夹不存在")
                        }
                    }
                    Target::Program { path, .. }
                    | Target::File { path }
                    | Target::Shortcut { path, .. } => {
                        let p = std::path::Path::new(path);
                        let low = path.to_ascii_lowercase();
                        let on_path =
                            matches!(resolved, Target::Program { .. }) && command_exists(path);
                        if low.starts_with("shell:")
                            || low.starts_with("::{")
                            || p.exists()
                            || on_path
                        {
                            None
                        } else {
                            Some("目标文件或快捷方式不存在")
                        }
                    }
                };
                if let Some(reason) = reason {
                    found.push(BrokenEntry {
                        id: entry.id,
                        name: entry.name,
                        target: entry.target.value().to_string(),
                        reason: reason.to_string(),
                        relocatable: entry.target.relocatable(),
                    });
                }
            }
            sink.send(AppEvent::BrokenCheckReady {
                epoch,
                items: found,
            });
        });
    }

    pub(super) fn sync_broken_ui(&self) {
        let items: Vec<_> = self
            .broken_items
            .iter()
            .map(|b| BrokenOption {
                id: b.id as i32,
                name: b.name.clone().into(),
                target: b.target.clone().into(),
                reason: b.reason.clone().into(),
                relocatable: b.relocatable,
            })
            .collect();
        self.main
            .set_broken_items(Rc::new(VecModel::from(items)).into());
    }

    pub(super) fn remove_broken_entry(&mut self, id: i64) {
        if self.editor.as_ref().is_some_and(|e| e.original.id == id) {
            self.status("请先关闭此项目的编辑窗口");
            return;
        }
        if let Err(e) = self.store.delete_entry(id) {
            self.status(e);
            return;
        }
        self.broken_items.retain(|item| item.id != id);
        self.sync_broken_ui();
        if self.main.get_selected_id() == id as i32 {
            self.main.set_selected_id(0);
        }
        self.report_refresh();
    }
}

pub(super) fn command_exists(command: &str) -> bool {
    let trimmed = command.trim().trim_matches('"');
    if trimmed.is_empty()
        || trimmed.contains('\\')
        || trimmed.contains('/')
        || trimmed.contains(':')
    {
        return false;
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    let has_ext = std::path::Path::new(trimmed).extension().is_some();
    let exts: Vec<String> = if has_ext {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect()
    };
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = if ext.is_empty() {
                dir.join(trimmed)
            } else {
                dir.join(format!("{trimmed}{ext}"))
            };
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}
