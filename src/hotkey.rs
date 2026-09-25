//! Validated, portable representation of a Windows global hotkey.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub modifiers: u32,
    pub key: u32,
    pub label: String,
}
pub const DEFAULT: &str = "Ctrl+Alt+Space";
/// Physical codes remain stable when Ctrl/Alt or an input method consumes text.
pub fn physical_name(code: &str) -> Option<String> {
    let name = code
        .strip_prefix("Key")
        .or_else(|| code.strip_prefix("Digit"))
        .unwrap_or(code);
    parse(&format!("Ctrl+{name}"))
        .ok()
        .map(|_| name.to_string())
}
pub fn parse(value: &str) -> Result<Shortcut, String> {
    let mut modifiers = 0;
    let mut key = None;
    let mut name = String::new();
    for token in value.split('+').map(str::trim) {
        let upper = token.to_ascii_uppercase();
        let modifier = match upper.as_str() {
            "CTRL" | "CONTROL" => 2,
            "ALT" => 1,
            "SHIFT" => 4,
            "WIN" | "META" => 8,
            _ => 0,
        };
        if modifier != 0 {
            modifiers |= modifier;
            continue;
        }
        if key.is_some() {
            return Err("只允许一个主按键".into());
        }
        let code = match upper.as_str() {
            "SPACE" | "空格" => {
                name = "Space".into();
                0x20
            }
            "ENTER" => {
                name = "Enter".into();
                0x0d
            }
            "HOME" => {
                name = "Home".into();
                0x24
            }
            "END" => {
                name = "End".into();
                0x23
            }
            _ if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric() => {
                name = upper.clone();
                u32::from(upper.as_bytes()[0])
            }
            _ if upper.starts_with('F') => {
                let n = upper[1..].parse::<u32>().map_err(|_| "无效功能键")?;
                if !(1..=24).contains(&n) {
                    return Err("功能键应为 F1–F24".into());
                }
                name = upper.clone();
                0x6f + n
            }
            _ => return Err("使用 Ctrl、Alt、Shift、Win 与字母、数字或功能键组合".into()),
        };
        key = Some(code);
    }
    let key = key.ok_or("请按下完整的组合键")?;
    if modifiers & (1 | 2 | 8) == 0 {
        return Err("请至少包含 Ctrl、Alt 或 Win".into());
    }
    let mut parts = Vec::new();
    for (bit, label) in [(2, "Ctrl"), (1, "Alt"), (4, "Shift"), (8, "Win")] {
        if modifiers & bit != 0 {
            parts.push(label.to_string());
        }
    }
    parts.push(name);
    Ok(Shortcut {
        modifiers,
        key,
        label: parts.join("+"),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_canonicalizes() {
        assert_eq!(physical_name("KeyK").as_deref(), Some("K"));
        assert_eq!(physical_name("Space").as_deref(), Some("Space"));
        assert_eq!(physical_name("F12").as_deref(), Some("F12"));
        assert!(physical_name("ControlLeft").is_none());
        assert_eq!(parse(" alt + ctrl + space ").unwrap().label, DEFAULT);
        assert_eq!(parse("Ctrl+Shift+F12").unwrap().key, 0x7b);
        for bad in ["A", "Shift+A", "Ctrl", "Ctrl+A+B", "Alt+F25"] {
            assert!(parse(bad).is_err());
        }
    }
}
