//! File classification runs off the event loop. Dropping never executes a target.
use crate::model::{Entry, Error, Result, ShowMode, Target, normalize_url};
use std::path::PathBuf;

pub fn file_entries(paths: Vec<(PathBuf, i64)>) -> Result<Vec<Entry>> {
    if paths.len() > 500 {
        return Err(Error::Invalid(
            crate::i18n::t(
                "每次最多添加 500 项",
                "You can add at most 500 items at once.",
            )
            .into(),
        ));
    }
    paths
        .into_iter()
        .map(|(path, collection_id)| {
            let metadata = std::fs::metadata(&path).map_err(|e| {
                Error::Invalid(format!(
                    "{}{}：{e}",
                    crate::i18n::t("无法读取 ", "Cannot read "),
                    path.display()
                ))
            })?;
            let value = path
                .to_str()
                .ok_or_else(|| {
                    Error::Invalid(
                        crate::i18n::t(
                            "路径包含无法表示的字符",
                            "Path contains unsupported characters.",
                        )
                        .into(),
                    )
                })?
                .to_string();
            let extension = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let target = if metadata.is_dir() {
                Target::Folder { path: value }
            } else {
                match extension.as_str() {
                    "exe" | "bat" | "cmd" | "msi" | "com" => Target::Program {
                        path: value,
                        arguments: String::new(),
                        working_dir: String::new(),
                        elevated: false,
                        show: ShowMode::default(),
                    },
                    "url" => match crate::platform::resolve_url_file(&path) {
                        Some(url) => Target::Url { url },
                        None => Target::Shortcut {
                            path: value,
                            show: ShowMode::default(),
                        },
                    },
                    "lnk" => crate::platform::resolve_lnk_file(&path).unwrap_or(Target::Shortcut {
                        path: value,
                        show: ShowMode::default(),
                    }),
                    _ => Target::File { path: value },
                }
            };
            let name = if metadata.is_dir() {
                path.file_name()
            } else {
                path.file_stem()
            }
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
            let entry = Entry {
                icon: crate::platform::shell_icon(&path),
                id: 0,
                collection_id,
                name,
                target,
                aliases: String::new(),
                notes: String::new(),
                position: 0,
            };
            entry.validate()?;
            Ok(entry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_without_executing_and_preserves_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = vec![(dir.path().to_path_buf(), 7)];
        for name in ["程序.EXE", "link.lnk", "web.url", "notes.txt"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"not executable").unwrap();
            paths.push((path, 7));
        }
        let entries = file_entries(paths).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.target.kind_index())
                .collect::<Vec<_>>(),
            [2, 0, 4, 4, 1]
        );
        assert!(entries.iter().all(|e| e.collection_id == 7));
        assert!(file_entries(vec![(dir.path().join("missing"), 7)]).is_err());

        let resolved_url = dir.path().join("bookmark.url");
        std::fs::write(
            &resolved_url,
            "[InternetShortcut]\r\nURL=https://example.com\r\n",
        )
        .unwrap();
        let resolved_lnk = crate::platform::create_shortcut_in(
            dir.path(),
            "工具",
            &Target::Program {
                path: "C:\\Windows\\System32\\notepad.exe".into(),
                arguments: "--help".into(),
                working_dir: "C:\\Windows".into(),
                elevated: false,
                show: ShowMode::Normal,
            },
        )
        .unwrap();
        let resolved = file_entries(vec![(resolved_url, 7), (resolved_lnk, 7)]).unwrap();
        assert_eq!(resolved[0].target.kind_index(), 3);
        assert_eq!(resolved[0].target.value(), "https://example.com");
        assert_eq!(resolved[1].target.kind_index(), 0);
        assert_eq!(
            resolved[1].target.value(),
            "C:\\Windows\\System32\\notepad.exe"
        );
    }
}

#[derive(Clone, Debug)]
pub enum ExternalDrop {
    Files(Vec<PathBuf>),
    Text(String),
}

pub fn url_entries(text: &str, collection_id: i64) -> Result<Vec<Entry>> {
    if text.len() > 1024 * 1024 {
        return Err(Error::Invalid(
            crate::i18n::t("拖入文本过长", "Dropped text is too large.").into(),
        ));
    }
    let lines: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();
    let mut entries = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // A dragged selection is "address\npage title", and the address may leave the scheme out.
        let Some(url) = normalize_url(line) else {
            continue;
        };
        let target = Target::Url { url };
        target.validate()?;
        let title = lines.get(i + 1).filter(|s| normalize_url(s).is_none());
        let cleaned_title: String = title
            .map(|t| t.chars().filter(|c| !c.is_control()).take(200).collect())
            .unwrap_or_default();
        let name: String = if cleaned_title.trim().is_empty() {
            line.chars().take(200).collect()
        } else {
            cleaned_title
        };
        let entry = Entry {
            id: 0,
            collection_id,
            target,
            name,
            aliases: String::new(),
            notes: String::new(),
            icon: vec![],
            position: 0,
        };
        entry.validate()?;
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(Error::Invalid(
            crate::i18n::t(
                "请拖入网址，可省略 https:// 直接写 example.com",
                "Drop a URL (you can omit https://, e.g. example.com).",
            )
            .into(),
        ));
    }
    if entries.len() > 500 {
        return Err(Error::Invalid(
            crate::i18n::t(
                "每次最多添加 500 项",
                "You can add at most 500 items at once.",
            )
            .into(),
        ));
    }
    Ok(entries)
}

#[cfg(test)]
mod url_tests {
    use super::*;
    #[test]
    fn browser_url_title_uri_lists_and_invalid_text() {
        let items = url_entries("https://example.com/路径\n网页标题", 2).unwrap();
        assert_eq!(items[0].name, "网页标题");
        assert_eq!(items[0].collection_id, 2);
        let bare = url_entries("example.com/路径\n网页标题", 2).unwrap();
        assert_eq!(
            bare[0].target.value(),
            "https://example.com/路径",
            "the scheme a browser tab leaves out is filled in"
        );
        assert_eq!(bare[0].name, "网页标题");
        assert_eq!(
            url_entries("www.example.com\nwww.example.com", 2)
                .unwrap()
                .len(),
            2,
            "two addresses are two items, not an address and its title"
        );
        assert_eq!(
            url_entries(
                "#comment\r\nhttps://example.com/a\r\nhttps://example.com/b",
                3
            )
            .unwrap()
            .len(),
            2
        );
        for invalid in [
            "javascript:alert(1)",
            "some text",
            "https://",
            "file:///c:/test",
        ] {
            assert!(url_entries(invalid, 2).is_err());
        }
    }
}
