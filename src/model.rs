use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("数据格式无效：{0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShowMode {
    #[default]
    Normal,
    Maximized,
    Minimized,
}
impl ShowMode {
    pub fn index(self) -> i32 {
        match self {
            Self::Normal => 0,
            Self::Maximized => 1,
            Self::Minimized => 2,
        }
    }
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Maximized,
            2 => Self::Minimized,
            _ => Self::Normal,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Program {
        path: String,
        arguments: String,
        working_dir: String,
        elevated: bool,
        show: ShowMode,
    },
    File {
        path: String,
    },
    Folder {
        path: String,
    },
    Url {
        url: String,
    },
    Shortcut {
        path: String,
        show: ShowMode,
    },
    App {
        aumid: String,
    },
}
/// Accept the shapes a person actually types or drags in: "example.com", "www.example.com/a?b=1"
/// and "localhost:8080" are all web addresses, and https is the scheme a browser tries first. Text
/// that cannot name a host stays None so the caller reports a format error instead of launching a
/// nonsense scheme. The typed text is kept as written after the scheme is filled in: running the
/// value through `Url::to_string` would add a trailing slash and punycode non-ASCII hosts.
pub fn normalize_url(input: &str) -> Option<String> {
    let text = input.trim().trim_matches('"').trim();
    if text.is_empty()
        || text.len() > 2040
        || text.contains('\\')
        || text.chars().any(|c| c.is_whitespace() || c == '\0')
    {
        return None;
    }
    let candidate = if text.contains("://") {
        text.to_string()
    } else {
        // A bare address is host[:port] and nothing else: no userinfo, and a colon that does not
        // start a numeric port means this is a scheme written without its slashes.
        let authority = text.split(['/', '?', '#']).next().unwrap_or(text);
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, port),
            None => (authority, ""),
        };
        if authority.contains('@')
            || (!port.is_empty() && !port.bytes().all(|b| b.is_ascii_digit()))
            || (host != "localhost" && !host.contains('.'))
        {
            return None;
        }
        format!("https://{text}")
    };
    let parsed = url::Url::parse(&candidate).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(candidate)
}
impl Target {
    pub fn value(&self) -> &str {
        match self {
            Self::Program { path, .. }
            | Self::File { path }
            | Self::Folder { path }
            | Self::Shortcut { path, .. } => path,
            Self::Url { url } => url,
            Self::App { aumid } => aumid,
        }
    }
    pub fn kind_index(&self) -> i32 {
        match self {
            Self::Program { .. } => 0,
            Self::File { .. } => 1,
            Self::Folder { .. } => 2,
            Self::Url { .. } => 3,
            Self::Shortcut { .. } => 4,
            Self::App { .. } => 5,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let value = self.value();
        if value.trim().is_empty() || value.contains('\0') || value.len() > 32760 {
            return Err(Error::Invalid(
                "请输入有效目标（不能包含空字符或超过长度限制）".into(),
            ));
        }
        match self {
            Self::Url { url } => {
                let parsed = url::Url::parse(url).map_err(|_| {
                    Error::Invalid("网址格式无效，可省略协议直接写 example.com".into())
                })?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    return Err(Error::Invalid("网址只接受 http:// 或 https://".into()));
                }
            }
            Self::Program {
                arguments,
                working_dir,
                ..
            } if (arguments.contains('\0')
                || working_dir.contains('\0')
                || arguments.len() > 32760
                || working_dir.len() > 32760) =>
            {
                return Err(Error::Invalid("参数或工作目录无效".into()));
            }
            _ => {}
        }
        // Do not probe filesystem availability: offline/network targets must remain editable.
        if matches!(
            self,
            Self::Program { .. } | Self::File { .. } | Self::Folder { .. } | Self::Shortcut { .. }
        ) {
            let bytes = value.as_bytes();
            let drive_absolute = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            if !drive_absolute
                && !value.starts_with("\\\\")
                && !value.starts_with(".\\")
                && !value.starts_with("..\\")
                && !value.starts_with("./")
                && !value.starts_with("../")
            {
                return Err(Error::Invalid(
                    "请选择绝对路径，例如 C:\\Apps\\app.exe".into(),
                ));
            }
        }
        Ok(())
    }
    /// Re-points a target at a new path, keeping every other field: relocating a broken link
    /// must not silently drop arguments, working directory, elevation or window mode.
    pub fn with_path(&self, path: &str) -> Result<Self> {
        let path = path.to_string();
        Ok(match self {
            Self::Program {
                arguments,
                working_dir,
                elevated,
                show,
                ..
            } => Self::Program {
                path,
                arguments: arguments.clone(),
                working_dir: working_dir.clone(),
                elevated: *elevated,
                show: *show,
            },
            Self::File { .. } => Self::File { path },
            Self::Folder { .. } => Self::Folder { path },
            Self::Shortcut { show, .. } => Self::Shortcut { path, show: *show },
            Self::Url { .. } | Self::App { .. } => {
                return Err(Error::Invalid(
                    "网址和应用标识没有可重新定位的文件路径".into(),
                ));
            }
        })
    }
    pub fn relocatable(&self) -> bool {
        !matches!(self, Self::Url { .. } | Self::App { .. })
    }
    pub fn duplicate_key(&self) -> String {
        match self {
            // URL path and query can be case-sensitive.
            Self::Url { url } => url::Url::parse(url)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| url.clone()),
            Self::Program {
                path,
                arguments,
                working_dir,
                elevated,
                show,
            } => format!(
                "{}|{}|{}|{}|{}",
                path.replace('/', "\\").to_lowercase(),
                arguments,
                working_dir.to_lowercase(),
                elevated,
                show.index()
            ),
            Self::Shortcut { path, show } => format!(
                "{}|{}|{}",
                self.kind_index(),
                path.replace('/', "\\").to_lowercase(),
                show.index()
            ),
            _ => format!(
                "{}|{}",
                self.kind_index(),
                self.value().replace('/', "\\").to_lowercase()
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub position: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Embedded PNG bytes; travels with database backups and JSON exports.
    #[serde(default)]
    pub icon: Vec<u8>,
    pub id: i64,
    pub collection_id: i64,
    pub name: String,
    pub aliases: String,
    pub notes: String,
    pub target: Target,
    pub position: i64,
}
impl Entry {
    pub fn validate(&self) -> Result<()> {
        if self.icon.len() > 256 * 1024 {
            return Err(Error::Invalid("图标数据过大".into()));
        }
        validate_name(&self.name)?;
        if self.aliases.len() > 8192 || self.notes.len() > 32760 {
            return Err(Error::Invalid("别名或备注过长".into()));
        }
        self.target.validate()
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 200 || name.contains('\0') {
        Err(Error::Invalid("名称不能为空，且最多 200 个字符".into()))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub hotkey: String,
    pub autostart: bool,
    pub double_click: bool,
    pub icon_size: u8,
    pub list_view: bool,
    pub edge_hide: bool,
    pub edge_strip_visible: bool,
    pub edge_strip_width_px: u32,
    pub hide_on_focus_loss: bool,
    pub hover_switch: bool,
    pub hover_delay_ms: u32,
    pub relative_paths: bool,
    pub hide_after_launch: bool,
    pub fullscreen_protection: bool,
    /// Opt-in local record of protection decisions; those rows can contain local paths.
    pub protection_log: bool,
    pub protected_apps: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: crate::hotkey::DEFAULT.into(),
            autostart: false,
            double_click: false,
            icon_size: 1,
            list_view: false,
            edge_hide: true,
            edge_strip_visible: true,
            edge_strip_width_px: 1,
            hide_on_focus_loss: false,
            hover_switch: false,
            hover_delay_ms: 350,
            relative_paths: false,
            hide_after_launch: false,
            fullscreen_protection: true,
            protection_log: false,
            protected_apps: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Export {
    pub format_version: u32,
    pub collections: Vec<Collection>,
    pub entries: Vec<Entry>,
    pub settings: Settings,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validate_without_accessing_targets() {
        assert!(
            Target::File {
                path: "\\\\offline\\share\\中文.txt".into()
            }
            .validate()
            .is_ok()
        );
        assert!(
            Target::File {
                path: "relative.exe".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Target::Url {
                url: "javascript:alert(1)".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Target::Url {
                url: "https://example.com/a".into()
            }
            .validate()
            .is_ok()
        );
    }
    /// What a person types in the address box, or drags in from a browser tab, has to be enough on
    /// its own; anything that is not a host must stay None so it is never launched.
    #[test]
    fn addresses_without_a_scheme_are_completed_as_https() {
        assert_eq!(
            normalize_url("example.com").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            normalize_url("  www.example.com/a?b=1#x  ").as_deref(),
            Some("https://www.example.com/a?b=1#x")
        );
        assert_eq!(
            normalize_url("localhost:8080").as_deref(),
            Some("https://localhost:8080")
        );
        assert_eq!(
            normalize_url("127.0.0.1:9/p").as_deref(),
            Some("https://127.0.0.1:9/p")
        );
        assert_eq!(
            normalize_url("\"http://example.com/\"").as_deref(),
            Some("http://example.com/"),
            "a written-out scheme is kept exactly as typed"
        );
        for rejected in [
            "",
            "   ",
            "notaurl",
            "C:\\Apps\\app.exe",
            "带 空格",
            "mailto:a@b.com",
            "ftp://example.com",
            "example.com:port",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                normalize_url(rejected),
                None,
                "{rejected} is not an address"
            );
        }
    }
    #[test]
    fn url_paths_remain_case_sensitive() {
        assert_ne!(
            Target::Url {
                url: "https://example.com/A".into()
            }
            .duplicate_key(),
            Target::Url {
                url: "https://example.com/a".into()
            }
            .duplicate_key()
        );
    }
    fn program(show: ShowMode) -> Target {
        Target::Program {
            path: "C:\\Apps\\tool.exe".into(),
            arguments: "--flag".into(),
            working_dir: "C:\\Apps".into(),
            elevated: false,
            show,
        }
    }
    #[test]
    fn window_mode_participates_in_duplicate_keys() {
        // Otherwise 普通 and 最小化 for the same exe would be rejected as the same entry.
        assert_ne!(
            program(ShowMode::Normal).duplicate_key(),
            program(ShowMode::Minimized).duplicate_key()
        );
        assert_eq!(
            program(ShowMode::Maximized).duplicate_key(),
            program(ShowMode::Maximized).duplicate_key()
        );
        assert_ne!(
            Target::Shortcut {
                path: "C:\\Links\\tool.lnk".into(),
                show: ShowMode::Normal
            }
            .duplicate_key(),
            Target::Shortcut {
                path: "C:\\Links\\tool.lnk".into(),
                show: ShowMode::Maximized
            }
            .duplicate_key()
        );
    }
    #[test]
    fn with_path_keeps_every_other_field_and_refuses_pathless_targets() {
        let moved = program(ShowMode::Minimized)
            .with_path("D:\\New\\tool.exe")
            .unwrap();
        assert_eq!(
            moved,
            Target::Program {
                path: "D:\\New\\tool.exe".into(),
                arguments: "--flag".into(),
                working_dir: "C:\\Apps".into(),
                elevated: false,
                show: ShowMode::Minimized
            }
        );
        let shortcut = Target::Shortcut {
            path: "C:\\Links\\old.lnk".into(),
            show: ShowMode::Maximized,
        }
        .with_path("C:\\Links\\new.lnk")
        .unwrap();
        assert_eq!(
            shortcut,
            Target::Shortcut {
                path: "C:\\Links\\new.lnk".into(),
                show: ShowMode::Maximized
            }
        );
        assert!(
            Target::Url {
                url: "https://example.com".into()
            }
            .with_path("C:\\x")
            .is_err()
        );
        assert!(
            Target::App {
                aumid: "Example.App!Entry".into()
            }
            .with_path("C:\\x")
            .is_err()
        );
        assert!(program(ShowMode::Normal).relocatable());
        assert!(
            !Target::App {
                aumid: "Example.App!Entry".into()
            }
            .relocatable()
        );
    }
}
