//! Runtime UI language selection (`zh-CN` / `en-US`) with lock-free lookup.
use std::sync::atomic::{AtomicU8, Ordering};

pub const LANGUAGE_SYSTEM: u8 = 0;
const LANGUAGE_ZH_CN: u8 = 1;
const LANGUAGE_EN_US: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Locale {
    ZhCn = 0,
    EnUs = 1,
}

static ACTIVE_LOCALE: AtomicU8 = AtomicU8::new(Locale::ZhCn as u8);

pub fn parse_locale_tag(tag: &str) -> Locale {
    let trimmed = tag.trim().to_ascii_lowercase();
    if trimmed == "zh" || trimmed.starts_with("zh-") || trimmed.starts_with("zh_") {
        Locale::ZhCn
    } else {
        Locale::EnUs
    }
}

fn resolve_locale(mode: u8, system: Locale) -> Locale {
    match mode {
        LANGUAGE_ZH_CN => Locale::ZhCn,
        LANGUAGE_EN_US => Locale::EnUs,
        _ => system,
    }
}

pub fn set_locale(locale: Locale) {
    ACTIVE_LOCALE.store(locale as u8, Ordering::Relaxed);
}

pub fn locale() -> Locale {
    if ACTIVE_LOCALE.load(Ordering::Relaxed) == Locale::EnUs as u8 {
        Locale::EnUs
    } else {
        Locale::ZhCn
    }
}

pub fn is_en() -> bool {
    locale() == Locale::EnUs
}

pub fn apply_language_mode(mode: u8) -> Locale {
    #[cfg(windows)]
    let system = crate::platform::system_locale();
    #[cfg(not(windows))]
    let system = Locale::ZhCn;

    let resolved = resolve_locale(mode, system);
    set_locale(resolved);
    resolved
}

#[inline]
pub fn t<'a>(zh: &'a str, en: &'a str) -> &'a str {
    t_for(locale(), zh, en)
}

#[inline]
pub fn t_for<'a>(locale: Locale, zh: &'a str, en: &'a str) -> &'a str {
    match locale {
        Locale::ZhCn => zh,
        Locale::EnUs => en,
    }
}

pub fn default_group_name() -> &'static str {
    t("常用", "Common")
}

pub fn default_category_name() -> &'static str {
    t("默认分类", "Default")
}

pub fn is_default_category_name(name: &str) -> bool {
    matches!(name.trim(), "默认分类" | "Default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bcp47_language_tags() {
        assert_eq!(parse_locale_tag("zh-CN"), Locale::ZhCn);
        assert_eq!(parse_locale_tag("zh-Hans-CN"), Locale::ZhCn);
        assert_eq!(parse_locale_tag("ZH_TW"), Locale::ZhCn);
        assert_eq!(parse_locale_tag("zh"), Locale::ZhCn);
        assert_eq!(parse_locale_tag("en-US"), Locale::EnUs);
        assert_eq!(parse_locale_tag("ja-JP"), Locale::EnUs);
        assert_eq!(parse_locale_tag(""), Locale::EnUs);
    }

    #[test]
    fn resolves_language_modes_without_mutating_global_state() {
        assert_eq!(resolve_locale(LANGUAGE_SYSTEM, Locale::EnUs), Locale::EnUs);
        assert_eq!(resolve_locale(LANGUAGE_SYSTEM, Locale::ZhCn), Locale::ZhCn);
        assert_eq!(resolve_locale(LANGUAGE_ZH_CN, Locale::EnUs), Locale::ZhCn);
        assert_eq!(resolve_locale(LANGUAGE_EN_US, Locale::ZhCn), Locale::EnUs);
        assert_eq!(t_for(Locale::ZhCn, "中文", "English"), "中文");
        assert_eq!(t_for(Locale::EnUs, "中文", "English"), "English");
        assert!(is_default_category_name("默认分类"));
        assert!(is_default_category_name("Default"));
        assert!(!is_default_category_name("Work"));
    }
}
