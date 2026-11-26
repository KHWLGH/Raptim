//! Current-user Windows `Run` registry autostart configuration and verification.
use super::*;

pub fn autostart_command(exe: &Path, data_dir: &Path) -> String {
    let clean_dir = crate::paths::normal_path(data_dir);
    format!(
        "\"{}\" --data-dir \"{}\" --silent",
        exe.display(),
        clean_dir.display()
    )
}

pub fn set_autostart(enabled: bool, data_dir: &Path) -> Result<(), String> {
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()
        .map_err(|e| e.to_string())?;
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = wide(&autostart_command(&exe, data_dir));
            let bytes =
                std::slice::from_raw_parts(command.as_ptr() as *const u8, command.len() * 2);
            RegSetValueExW(key, w!("Raptim"), Some(0), REG_SZ, Some(bytes))
                .ok()
                .map_err(|e| e.to_string())
        } else {
            let code = RegDeleteValueW(key, w!("Raptim"));
            if code == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                code.ok().map_err(|e| e.to_string())
            }
        };
        let _ = RegCloseKey(key);
        result
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
        let mut run_key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut run_key,
        )
        .is_err()
        {
            return Some("开机启动项未写入系统注册表，请重新开关生效".into());
        }
        let mut reg_type = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        let query = RegQueryValueExW(
            run_key,
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
                run_key,
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
        let _ = RegCloseKey(run_key);
        let Some(actual) = actual else {
            return Some("开机启动项未写入系统注册表，请重新开关生效".into());
        };
        if !actual.trim().eq_ignore_ascii_case(expected.trim()) {
            return Some("程序路径、数据目录或静默参数已变更，请重新开关「开机启动」更新".into());
        }
        let mut approved = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"),
            Some(0),
            KEY_QUERY_VALUE,
            &mut approved,
        )
        .is_ok()
        {
            let mut buf = [0u8; 16];
            let mut read = buf.len() as u32;
            let mut kind = REG_VALUE_TYPE::default();
            if RegQueryValueExW(
                approved,
                w!("Raptim"),
                None,
                Some(&mut kind),
                Some(buf.as_mut_ptr()),
                Some(&mut read),
            )
            .is_ok()
                && read >= 1
                && (buf[0] & 1 == 1)
            {
                let _ = RegCloseKey(approved);
                return Some("系统任务管理器已禁用开机启动，请在任务管理器启用或重新开关".into());
            }
            let _ = RegCloseKey(approved);
        }
    }
    None
}
