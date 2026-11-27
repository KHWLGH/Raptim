//! Current-user Windows `Run` registry autostart configuration and verification.
use super::*;

use windows::Win32::System::Registry::{HKEY, RegCloseKey};

struct OwnedRegKey(HKEY);
impl Drop for OwnedRegKey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

fn quote_win_arg(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in text.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            for _ in 0..(backslashes * 2 + 1) {
                out.push('\\');
            }
            out.push('"');
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                out.push('\\');
            }
            out.push(ch);
            backslashes = 0;
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

fn autostart_command(exe: &Path, data_dir: &Path) -> String {
    let clean_dir = crate::paths::normal_path(data_dir);
    format!(
        "{} --data-dir {} --silent",
        quote_win_arg(&exe.to_string_lossy()),
        quote_win_arg(&clean_dir.to_string_lossy()),
    )
}

pub fn set_autostart(enabled: bool, data_dir: &Path) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut raw_key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut raw_key,
            None,
        )
        .ok()
        .map_err(|e| e.to_string())?;
        let key = OwnedRegKey(raw_key);
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&autostart_command(&exe, data_dir));
            let bytes =
                std::slice::from_raw_parts(command.as_ptr() as *const u8, command.len() * 2);
            RegSetValueExW(key.0, w!("Raptim"), Some(0), REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| e.to_string())?;
            let mut raw_approved = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"),
                Some(0),
                KEY_SET_VALUE,
                &mut raw_approved,
            )
            .is_ok()
            {
                let approved = OwnedRegKey(raw_approved);
                let _ = RegDeleteValueW(approved.0, w!("Raptim"));
            }
            Ok(())
        } else {
            let code = RegDeleteValueW(key.0, w!("Raptim"));
            if code == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                code.ok().map_err(|e| e.to_string())
            }
        }
    }
}

/// Check whether the Windows `Run` entry and Task Manager approval match the saved preference.
pub fn autostart_mismatch(data_dir: &Path, expected_enabled: bool) -> Option<String> {
    use windows::Win32::System::Registry::*;
    if !expected_enabled {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let expected = autostart_command(&exe, data_dir);
    unsafe {
        let mut raw_run_key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut raw_run_key,
        )
        .is_err()
        {
            return Some(
                crate::i18n::t(
                    "开机启动项未写入系统注册表，请重新开关生效",
                    "Startup entry missing from registry; toggle Autostart again to apply.",
                )
                .into(),
            );
        }
        let run_key = OwnedRegKey(raw_run_key);
        let mut reg_type = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let query = RegQueryValueExW(
            run_key.0,
            w!("Raptim"),
            None,
            Some(&mut reg_type),
            None,
            Some(&mut size),
        );
        let actual = if query.is_ok() && reg_type == REG_SZ && size >= 2 {
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            let mut read = size;
            if RegQueryValueExW(
                run_key.0,
                w!("Raptim"),
                None,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut read),
            )
            .is_ok()
            {
                let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                Some(String::from_utf16_lossy(&buf[..end]))
            } else {
                None
            }
        } else {
            None
        };
        drop(run_key);
        let Some(actual) = actual else {
            return Some(
                crate::i18n::t(
                    "开机启动项未写入系统注册表，请重新开关生效",
                    "Startup entry missing from registry; toggle Autostart again to apply.",
                )
                .into(),
            );
        };
        if !actual.trim().eq_ignore_ascii_case(expected.trim()) {
            return Some(
                crate::i18n::t(
                    "程序路径、数据目录或静默参数已变更，请重新开关「开机启动」更新",
                    "Executable, data dir, or flags changed; toggle Autostart to update.",
                )
                .into(),
            );
        }
        let mut raw_approved = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut raw_approved,
        )
        .is_ok()
        {
            let approved = OwnedRegKey(raw_approved);
            let mut buf = [0u8; 16];
            let mut read = buf.len() as u32;
            let mut kind = REG_VALUE_TYPE::default();
            if RegQueryValueExW(
                approved.0,
                w!("Raptim"),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr()),
                Some(&mut read),
            )
            .is_ok()
                && kind == REG_BINARY
                && read >= 1
                && (buf[0] & 1 == 1)
            {
                return Some(
                    crate::i18n::t(
                        "系统任务管理器已禁用开机启动，请在任务管理器启用或重新开关",
                        "Startup disabled in Task Manager; re-enable there or toggle Autostart.",
                    )
                    .into(),
                );
            }
        }
    }
    None
}
