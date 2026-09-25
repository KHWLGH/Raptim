use crate::model::Entry;
use pinyin::ToPinyin;
use std::collections::HashMap;

#[derive(Default)]
pub struct SearchIndex {
    documents: HashMap<i64, Document>,
}
struct Document {
    source: (String, String, String),
    names: Vec<String>,
    full: Vec<String>,
    initials: Vec<String>,
    notes: String,
}
fn romanize(text: &str) -> (String, String) {
    let mut full = String::new();
    let mut initials = String::new();
    for c in text.chars() {
        if let Some(p) = c.to_pinyin() {
            full.push_str(p.plain());
            initials.push(p.plain().chars().next().unwrap());
        } else {
            full.push(c);
            initials.push(c);
        }
    }
    (full.to_lowercase(), initials.to_lowercase())
}
impl SearchIndex {
    pub fn sync(&mut self, entries: &[Entry]) {
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| e.id).collect();
        self.documents.retain(|id, _| ids.contains(id));
        for e in entries {
            // Compare before cloning: `source` exists only to detect a change, and building it
            // up front meant three allocations per entry on every refresh.
            if let Some(d) = self.documents.get(&e.id)
                && d.source.0 == e.name
                && d.source.1 == e.aliases
                && d.source.2 == e.notes
            {
                continue;
            }
            let source = (e.name.clone(), e.aliases.clone(), e.notes.clone());
            let names: Vec<_> = std::iter::once(e.name.to_lowercase())
                .chain(e.aliases.split_whitespace().map(str::to_lowercase))
                .collect();
            let (full, initials) = names.iter().map(|n| romanize(n)).unzip();
            self.documents.insert(
                e.id,
                Document {
                    source,
                    names,
                    full,
                    initials,
                    notes: e.notes.to_lowercase(),
                },
            );
        }
    }
    pub fn query(&self, query: &str, entries: &[Entry]) -> Vec<i64> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let words: Vec<_> = query.split_whitespace().collect();
        let mut found = Vec::new();
        for e in entries {
            let Some(d) = self.documents.get(&e.id) else {
                continue;
            };
            let mut score = 0;
            let mut matched = true;
            for word in &words {
                let rank = if d.names.iter().any(|v| v == word) {
                    0
                } else if d.names.iter().any(|v| v.starts_with(word)) {
                    1
                } else if d.names.iter().any(|v| v.contains(word)) {
                    2
                } else if d.full.iter().any(|v| v.contains(word)) {
                    3
                } else if d.initials.iter().any(|v| v.contains(word)) {
                    4
                } else if d.notes.contains(word) {
                    5
                } else {
                    matched = false;
                    break;
                };
                score += rank;
            }
            if matched {
                found.push((
                    !d.names.iter().any(|n| n == &query),
                    score,
                    e.position,
                    e.id,
                ));
            }
        }
        found.sort_unstable();
        found.into_iter().map(|(_, _, _, id)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Target;
    #[test]
    fn search_excludes_target_paths() {
        let entries = vec![Entry {
            id: 1,
            collection_id: 2,
            name: "画图工具".into(),
            aliases: "paint DRAW".into(),
            notes: "工作".into(),
            target: crate::model::Target::File {
                path: "C:\\secret.txt".into(),
            },
            icon: vec![],
            position: 0,
        }];
        let mut index = SearchIndex::default();
        index.sync(&entries);
        assert_eq!(index.query("draw 工作", &entries), vec![1]);
        assert!(index.query("secret", &entries).is_empty());
    }
    #[test]
    fn chinese_pinyin_tokens_ranking_and_incremental_update() {
        let mut entries = vec![Entry {
            id: 1,
            collection_id: 1,
            name: "计算器".into(),
            aliases: "calc".into(),
            notes: "工作 工具".into(),
            target: Target::Url {
                url: "https://example.com".into(),
            },
            position: 0,
            icon: vec![],
        }];
        let mut index = SearchIndex::default();
        index.sync(&entries);
        for q in ["计算", "jisuanqi", "jsq", "CALC 工作"] {
            assert_eq!(index.query(q, &entries), [1]);
        }
        assert!(index.query("calc 游戏", &entries).is_empty());
        entries[0].name = "终端".into();
        index.sync(&entries);
        assert!(index.query("jsq", &entries).is_empty());
        assert_eq!(index.query("zd", &entries), [1]);
    }
}
