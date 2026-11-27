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
fn romanize(text: &str) -> (String, Vec<String>) {
    let mut full = String::new();
    let mut char_initials = String::new();
    let mut word_initials = String::new();
    let mut camel_initials = String::new();
    let mut at_word_start = true;
    let mut prev_ascii_lower = false;

    for c in text.chars() {
        if let Some(p) = c.to_pinyin() {
            let plain = p.plain();
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                char_initials.push(first);
                word_initials.push(first);
                camel_initials.push(first);
            }
            at_word_start = true;
            prev_ascii_lower = false;
        } else {
            for low in c.to_lowercase() {
                full.push(low);
                char_initials.push(low);
            }
            if c.is_alphanumeric() {
                let low = c.to_lowercase().next().unwrap_or(c);
                if at_word_start {
                    word_initials.push(low);
                    camel_initials.push(low);
                } else if c.is_uppercase() && prev_ascii_lower {
                    camel_initials.push(low);
                }
                at_word_start = false;
                prev_ascii_lower = c.is_ascii_lowercase();
            } else {
                at_word_start = true;
                prev_ascii_lower = false;
            }
        }
    }
    let mut initials = vec![char_initials];
    if !word_initials.is_empty() && !initials.contains(&word_initials) {
        initials.push(word_initials);
    }
    if !camel_initials.is_empty() && !initials.contains(&camel_initials) {
        initials.push(camel_initials);
    }
    (full, initials)
}

fn split_aliases(aliases: &str) -> impl Iterator<Item = &str> {
    aliases
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '，' | ';' | '；' | '、' | '|'))
        .filter(|s| !s.is_empty())
}

impl SearchIndex {
    pub fn sync(&mut self, entries: &[Entry]) {
        if !self.documents.is_empty() {
            let ids: std::collections::HashSet<_> = entries.iter().map(|e| e.id).collect();
            self.documents.retain(|id, _| ids.contains(id));
        }
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
            let raw_tokens: Vec<&str> = std::iter::once(e.name.as_str())
                .chain(split_aliases(&e.aliases))
                .collect();
            let names: Vec<String> = raw_tokens.iter().map(|s| s.to_lowercase()).collect();
            let mut full = Vec::with_capacity(raw_tokens.len());
            let mut initials = Vec::new();
            for token in &raw_tokens {
                let (f, init_list) = romanize(token);
                full.push(f);
                for init in init_list {
                    if !initials.contains(&init) {
                        initials.push(init);
                    }
                }
            }
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
    #[test]
    fn english_word_and_camel_initials_and_punctuation_separated_aliases() {
        let entries = vec![
            Entry {
                id: 1,
                collection_id: 1,
                name: "Visual Studio Code".into(),
                aliases: "编辑器,vscode，代码;ide".into(),
                notes: String::new(),
                target: Target::Url {
                    url: "https://code.visualstudio.com".into(),
                },
                position: 0,
                icon: vec![],
            },
            Entry {
                id: 2,
                collection_id: 1,
                name: "GitHub Desktop".into(),
                aliases: String::new(),
                notes: String::new(),
                target: Target::Url {
                    url: "https://desktop.github.com".into(),
                },
                position: 1,
                icon: vec![],
            },
        ];
        let mut index = SearchIndex::default();
        index.sync(&entries);
        assert_eq!(index.query("vsc", &entries), [1]);
        assert_eq!(index.query("vscode", &entries), [1]);
        assert_eq!(index.query("bjq", &entries), [1]);
        assert_eq!(index.query("ide", &entries), [1]);
        assert_eq!(index.query("gd", &entries), [2]);
        assert_eq!(index.query("ghd", &entries), [2]);
    }
}
