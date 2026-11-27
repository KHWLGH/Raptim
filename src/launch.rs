//! What a launch attempt reported, and whether the user can act on it.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    Cancelled,
    Denied,
    Missing,
    NotReady,
    Offline,
    NoAssociation,
    AppNotInstalled,
    /// Target rejected by validation before the Shell was asked; keeps the field-level text.
    Invalid(String),
    Other(String),
}

impl LaunchError {
    /// `ShellExecuteExW` reports Win32 error codes in the low word of its `HRESULT`.
    pub fn from_os_error(code: u32) -> Self {
        match code {
            1223 => Self::Cancelled,
            5 => Self::Denied,
            // 2/3 not found, 123/161 malformed or removed device path: the target is gone.
            2 | 3 | 123 | 161 => Self::Missing,
            // 53/67: the share itself is unreachable right now, the target may still exist.
            53 | 67 => Self::Offline,
            // 21: drive present but not ready, typically a removable disk that is pulled.
            21 => Self::NotReady,
            1155 => Self::NoAssociation,
            _ => Self::Other(format!(
                "{}{code})",
                crate::i18n::t("启动失败（系统错误 ", "Launch failed (OS error ")
            )),
        }
    }

    pub fn message(&self) -> String {
        self.message_for(crate::i18n::locale())
    }

    pub fn message_for(&self, locale: crate::i18n::Locale) -> String {
        let s = |zh, en| crate::i18n::t_for(locale, zh, en);
        match self {
            Self::Cancelled => {
                s("已取消管理员授权", "Administrator elevation was cancelled.").into()
            }
            Self::Denied => s(
                "访问被拒绝，请检查目标权限",
                "Access denied. Check target permissions.",
            )
            .into(),
            Self::Missing => s(
                "找不到目标，可能已被删除或移动，可重新定位",
                "Target not found. It may have been moved or deleted — you can relocate it.",
            )
            .into(),
            Self::NotReady => s(
                "可移动设备未就绪，请插入后重试",
                "Removable drive is not ready. Reconnect and try again.",
            )
            .into(),
            Self::Offline => s(
                "网络位置暂不可用，请检查连接后重试",
                "Network location is unavailable. Check connection and try again.",
            )
            .into(),
            Self::NoAssociation => s(
                "没有可以打开此文件的关联应用",
                "No associated application can open this file.",
            )
            .into(),
            Self::AppNotInstalled => s(
                "应用未安装或已被移除，请在编辑中核对应用标识",
                "App is not installed or was removed. Check its AUMID in Edit.",
            )
            .into(),
            Self::Invalid(text) | Self::Other(text) => text.clone(),
        }
    }

    /// Only the classes that mean "the path we have does not resolve right now" are worth
    /// offering a new path for; an unclassified system failure is not.
    pub fn relocatable(&self) -> bool {
        matches!(
            self,
            Self::Missing | Self::NotReady | Self::Offline | Self::Invalid(_)
        )
    }
}

/// The hand-off from a worker thread back to the view. The entry's target kind is looked up
/// from the model again, so only what the user reads and decides travels here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Failure {
    pub entry: i64,
    pub message: String,
    pub relocatable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_errors_decode_to_distinct_classes() {
        for code in [2, 3, 123, 161] {
            assert_eq!(LaunchError::from_os_error(code), LaunchError::Missing);
        }
        for code in [53, 67] {
            assert_eq!(LaunchError::from_os_error(code), LaunchError::Offline);
        }
        assert_eq!(LaunchError::from_os_error(21), LaunchError::NotReady);
        assert_eq!(LaunchError::from_os_error(1223), LaunchError::Cancelled);
        assert_eq!(LaunchError::from_os_error(5), LaunchError::Denied);
        assert_eq!(LaunchError::from_os_error(1155), LaunchError::NoAssociation);
        assert!(matches!(
            LaunchError::from_os_error(9999),
            LaunchError::Other(_)
        ));
    }

    #[test]
    fn only_target_and_location_problems_offer_relocation() {
        for error in [
            LaunchError::Missing,
            LaunchError::NotReady,
            LaunchError::Offline,
            LaunchError::Invalid("目标无效".into()),
        ] {
            assert!(error.relocatable(), "{error:?}");
        }
        for error in [
            LaunchError::Cancelled,
            LaunchError::Denied,
            LaunchError::NoAssociation,
            LaunchError::AppNotInstalled,
            LaunchError::Other("启动失败".into()),
        ] {
            assert!(!error.relocatable(), "{error:?}");
        }
        assert!(LaunchError::Missing.message().contains("重新定位"));
        assert!(
            LaunchError::Missing
                .message_for(crate::i18n::Locale::EnUs)
                .contains("relocate")
        );
        assert_ne!(
            LaunchError::Missing.message(),
            LaunchError::Offline.message(),
            "deleted and temporarily unavailable must not share one sentence"
        );
    }
}
