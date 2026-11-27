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
    let name = match code {
        "Backquote" => "`",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        other => other
            .strip_prefix("Key")
            .or_else(|| other.strip_prefix("Digit"))
            .unwrap_or(other),
    };
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
            return Err(
                crate::i18n::t("只允许一个主按键", "Only one primary key is allowed.").into(),
            );
        }
        let code = match upper.as_str() {
            "SPACE" | "空格" => {
                name = "Space".into();
                0x20
            }
            "TAB" => {
                name = "Tab".into();
                0x09
            }
            "ENTER" => {
                name = "Enter".into();
                0x0d
            }
            "INSERT" => {
                name = "Insert".into();
                0x2d
            }
            "DELETE" => {
                name = "Delete".into();
                0x2e
            }
            "HOME" => {
                name = "Home".into();
                0x24
            }
            "END" => {
                name = "End".into();
                0x23
            }
            "PAGEUP" => {
                name = "PageUp".into();
                0x21
            }
            "PAGEDOWN" => {
                name = "PageDown".into();
                0x22
            }
            "`" | "BACKQUOTE" => {
                name = "`".into();
                0xc0
            }
            "-" | "MINUS" => {
                name = "-".into();
                0xbd
            }
            "=" | "EQUAL" => {
                name = "=".into();
                0xbb
            }
            "[" | "BRACKETLEFT" => {
                name = "[".into();
                0xdb
            }
            "]" | "BRACKETRIGHT" => {
                name = "]".into();
                0xdd
            }
            "\\" | "BACKSLASH" => {
                name = "\\".into();
                0xdc
            }
            ";" | "SEMICOLON" => {
                name = ";".into();
                0xba
            }
            "'" | "QUOTE" => {
                name = "'".into();
                0xde
            }
            "," | "COMMA" => {
                name = ",".into();
                0xbc
            }
            "." | "PERIOD" => {
                name = ".".into();
                0xbe
            }
            "/" | "SLASH" => {
                name = "/".into();
                0xbf
            }
            _ if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric() => {
                name = upper.clone();
                u32::from(upper.as_bytes()[0])
            }
            _ if upper.starts_with("NUMPAD") => {
                let digit = upper["NUMPAD".len()..].parse::<u32>().ok();
                let Some(digit) = digit.filter(|d| *d <= 9) else {
                    return Err(crate::i18n::t("无效小键盘数字键", "Invalid numpad digit.").into());
                };
                name = format!("Numpad{digit}");
                0x60 + digit
            }
            _ if upper.starts_with('F') => {
                let n = upper[1..]
                    .parse::<u32>()
                    .map_err(|_| crate::i18n::t("无效功能键", "Invalid function key."))?;
                if !(1..=24).contains(&n) {
                    return Err(crate::i18n::t(
                        "功能键应为 F1–F24",
                        "Function keys must be F1–F24.",
                    )
                    .into());
                }
                name = upper.clone();
                0x6f + n
            }
            _ => {
                return Err(crate::i18n::t(
                    "使用 Ctrl、Alt、Shift、Win 与常用按键或功能键组合",
                    "Combine Ctrl, Alt, Shift, or Win with a standard or function key.",
                )
                .into());
            }
        };
        key = Some(code);
    }
    let key = key
        .ok_or_else(|| crate::i18n::t("请按下完整的组合键", "Press a complete key combination."))?;
    if modifiers & (1 | 2 | 8) == 0 {
        return Err(crate::i18n::t(
            "请至少包含 Ctrl、Alt 或 Win",
            "Include at least Ctrl, Alt, or Win.",
        )
        .into());
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
        assert_eq!(physical_name("Backquote").as_deref(), Some("`"));
        assert_eq!(physical_name("Semicolon").as_deref(), Some(";"));
        assert_eq!(physical_name("Numpad1").as_deref(), Some("Numpad1"));
        assert!(physical_name("ControlLeft").is_none());
        assert_eq!(parse(" alt + ctrl + space ").unwrap().label, DEFAULT);
        assert_eq!(parse("Ctrl+Shift+F12").unwrap().key, 0x7b);
        assert_eq!(parse("Ctrl+`").unwrap().key, 0xc0);
        assert_eq!(parse("Alt+Insert").unwrap().key, 0x2d);
        assert_eq!(
            parse("Ctrl+Shift+Numpad5").unwrap().label,
            "Ctrl+Shift+Numpad5"
        );
        for bad in [
            "A",
            "Shift+A",
            "Ctrl",
            "Ctrl+A+B",
            "Alt+F25",
            "Ctrl+Numpad10",
        ] {
            assert!(parse(bad).is_err());
        }
    }
}
