use super::*;
use windows::core::{Interface, PCSTR};

struct Pidl(*mut Common::ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}

pub fn known_folder_path(folder: &windows::core::GUID) -> Result<std::path::PathBuf, String> {
    let _com = ComGuard::new()?;
    unsafe {
        let p = SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let value = p.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(p.0.cast()));
        Ok(std::path::PathBuf::from(value?))
    }
}

pub fn desktop_shortcut(name: &str, target: &Target) -> Result<std::path::PathBuf, String> {
    let desktop = known_folder_path(&FOLDERID_Desktop)?;
    create_shortcut_in(&desktop, name, target)
}

fn decode_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Read an `.url` Internet Shortcut file without executing anything.
pub fn resolve_url_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        let words: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line.eq_ignore_ascii_case("[InternetShortcut]");
            continue;
        }
        if in_section
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("URL")
        {
            return crate::model::normalize_url(value);
        }
    }
    None
}

/// Inspect a `.lnk` Shell link offline without UI or search heuristics. Returns `None` for MSI
/// advertised shortcuts, virtual shell targets, or unparseable files so the caller can keep `Shortcut`.
pub fn resolve_lnk_file(path: &Path) -> Option<Target> {
    if !path.is_file() {
        return None;
    }
    let _com = ComGuard::new().ok()?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        persist
            .Load(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), STGM_READ)
            .ok()?;
        let flags = link
            .cast::<IShellLinkDataList>()
            .ok()
            .and_then(|data| data.GetFlags().ok())
            .unwrap_or(0);
        // MSI advertised (Darwin) links resolve through installer tokens rather than a normal exe.
        if flags & SLDF_HAS_DARWINID.0 as u32 != 0 {
            return None;
        }
        let mut target_buf = [0u16; 32768];
        if link
            .GetPath(&mut target_buf, std::ptr::null_mut(), 0)
            .is_err()
        {
            return None;
        }
        let target_path = decode_wide(&target_buf).trim().to_string();
        if target_path.is_empty() {
            return None;
        }
        let mut args_buf = [0u16; 4096];
        let arguments = if link.GetArguments(&mut args_buf).is_ok() {
            decode_wide(&args_buf).trim().to_string()
        } else {
            String::new()
        };
        let mut dir_buf = [0u16; 4096];
        let working_dir = if link.GetWorkingDirectory(&mut dir_buf).is_ok() {
            decode_wide(&dir_buf).trim().to_string()
        } else {
            String::new()
        };
        let show = match link.GetShowCmd() {
            Ok(cmd) if cmd == SW_SHOWMAXIMIZED => ShowMode::Maximized,
            Ok(cmd) if cmd == SW_SHOWMINIMIZED || cmd == SW_SHOWMINNOACTIVE => ShowMode::Minimized,
            _ => ShowMode::Normal,
        };
        let elevated = flags & SLDF_RUNAS_USER.0 as u32 != 0;
        let ext = Path::new(&target_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_dir = Path::new(&target_path).is_dir();
        let target = if is_dir && arguments.is_empty() && !elevated && show == ShowMode::Normal {
            Target::Folder { path: target_path }
        } else if matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com" | "msi")
            || !arguments.is_empty()
            || !working_dir.is_empty()
            || elevated
            || show != ShowMode::Normal
        {
            Target::Program {
                path: target_path,
                arguments,
                working_dir,
                elevated,
                show,
            }
        } else {
            Target::File { path: target_path }
        };
        target.validate().ok()?;
        Some(target)
    }
}

/// Reserve a unique filename before writing. Never replace an existing desktop file.
pub fn create_shortcut_in(
    directory: &Path,
    name: &str,
    target: &Target,
) -> Result<std::path::PathBuf, String> {
    target.validate().map_err(|e| e.to_string())?;
    let _com = ComGuard::new()?;
    let safe: String = name
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(80)
        .collect();
    let mut safe = safe.trim().trim_end_matches('.').to_string();
    if safe.is_empty() {
        safe = crate::i18n::t("Raptim 快捷方式", "Raptim Shortcut").into();
    }
    let stem = safe.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        safe.insert(0, '_');
    }
    let ext = if matches!(target, Target::Url { .. }) {
        "url"
    } else {
        "lnk"
    };
    let (path, file) = (0..10000)
        .find_map(|n| {
            let path = directory.join(if n == 0 {
                format!("{safe}.{ext}")
            } else {
                format!("{safe} ({n}).{ext}")
            });
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => Some(Ok((path, file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e.to_string())),
            }
        })
        .unwrap_or_else(|| {
            Err(crate::i18n::t("同名快捷方式过多", "Too many shortcuts with the same name").into())
        })?;
    let result: Result<(), String> = (|| {
        if let Target::Url { url } = target {
            use std::io::Write;
            let url = url::Url::parse(url).map_err(|e| e.to_string())?;
            let mut file = file;
            write!(file, "[InternetShortcut]\r\nURL={url}\r\n").map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
        } else {
            drop(file);
            unsafe {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| e.to_string())?;
                let persist: IPersistFile = link.cast().map_err(|e| e.to_string())?;
                let value = wide(target.value());
                match target {
                    Target::Shortcut { .. } => persist
                        .Load(PCWSTR(value.as_ptr()), STGM_READ)
                        .map_err(|e| e.to_string())?,
                    Target::App { aumid } => {
                        let value = wide(&format!("shell:AppsFolder\\{aumid}"));
                        let mut id = std::ptr::null_mut();
                        SHParseDisplayName(PCWSTR(value.as_ptr()), None, &mut id, 0, None)
                            .map_err(|e| e.to_string())?;
                        let id = Pidl(id);
                        link.SetIDList(id.0).map_err(|e| e.to_string())?;
                    }
                    _ => link
                        .SetPath(PCWSTR(value.as_ptr()))
                        .map_err(|e| e.to_string())?,
                }
                if let Target::Program {
                    arguments,
                    working_dir,
                    elevated,
                    ..
                } = target
                {
                    link.SetArguments(PCWSTR(wide(arguments).as_ptr()))
                        .map_err(|e| e.to_string())?;
                    if !working_dir.is_empty() {
                        link.SetWorkingDirectory(PCWSTR(wide(working_dir).as_ptr()))
                            .map_err(|e| e.to_string())?;
                    }
                    if *elevated {
                        let data: IShellLinkDataList = link.cast().map_err(|e| e.to_string())?;
                        data.SetFlags(
                            data.GetFlags().map_err(|e| e.to_string())? | SLDF_RUNAS_USER.0 as u32,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
                if let Target::Program { show, .. } | Target::Shortcut { show, .. } = target {
                    link.SetShowCmd(show_cmd(*show))
                        .map_err(|e| e.to_string())?;
                }
                link.SetDescription(PCWSTR(wide(name).as_ptr()))
                    .map_err(|e| e.to_string())?;
                persist
                    .Save(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), true)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

struct ShellMenu {
    v2: Option<IContextMenu2>,
    v3: Option<IContextMenu3>,
}
unsafe extern "system" fn shell_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
        ) {
            let menu = &*(data as *const ShellMenu);
            let mut result = LRESULT(0);
            if menu
                .v3
                .as_ref()
                .is_some_and(|m| m.HandleMenuMsg2(msg, wp, lp, Some(&mut result)).is_ok())
            {
                return result;
            }
            if msg != WM_MENUCHAR
                && menu
                    .v2
                    .as_ref()
                    .is_some_and(|m| m.HandleMenuMsg(msg, wp, lp).is_ok())
            {
                return LRESULT(0);
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
pub fn shell_context_menu(window: &slint::Window, path: &Path) -> Result<(), String> {
    let _com = ComGuard::new()?;
    let hwnd =
        native_handle(window).ok_or_else(|| crate::i18n::t("窗口未就绪", "Window is not ready"))?;
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(
            PCWSTR(wide(&path.to_string_lossy()).as_ptr()),
            None,
            &mut pidl,
            0,
            None,
        )
        .map_err(|e| e.to_string())?;
        let pidl = Pidl(pidl);
        let mut child = std::ptr::null_mut();
        let folder: IShellFolder =
            SHBindToParent(pidl.0, Some(&mut child)).map_err(|e| e.to_string())?;
        // `child` is not an allocation of its own: measured on Windows 11 it is `pidl + 440` every
        // time, an interior pointer into the block above. The Shell's documentation for
        // `SHBindToParent`'s `ppidlLast` says the caller frees it, and doing that here aborts the
        // process with STATUS_HEAP_CORRUPTION. So this is deliberately not wrapped in `Pidl`, and
        // `pidl` has to outlive every use of `child` below.
        let context: IContextMenu = folder
            .GetUIObjectOf(hwnd, &[child], None)
            .map_err(|e| e.to_string())?;
        let menu = CreatePopupMenu().map_err(|e| e.to_string())?;
        let result = (|| {
            context
                .QueryContextMenu(menu, 0, 1, 0x7fff, CMF_NORMAL)
                .ok()
                .map_err(|e| e.to_string())?;
            let bridge = ShellMenu {
                v2: context.cast().ok(),
                v3: context.cast().ok(),
            };
            if !SetWindowSubclass(
                hwnd,
                Some(shell_proc),
                0xD144,
                &bridge as *const ShellMenu as usize,
            )
            .as_bool()
            {
                return Err(crate::i18n::t(
                    "无法连接系统菜单",
                    "Failed to attach Windows context menu",
                )
                .into());
            }
            let (x, y) = cursor();
            let _ = SetForegroundWindow(hwnd);
            let command = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0,
                x,
                y,
                hwnd,
                None,
            )
            .0 as u32;
            let _ = RemoveWindowSubclass(hwnd, Some(shell_proc), 0xD144);
            if command != 0 {
                context
                    .InvokeCommand(&CMINVOKECOMMANDINFO {
                        cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                        hwnd,
                        lpVerb: PCSTR((command - 1) as usize as *const u8),
                        nShow: SW_SHOWNORMAL.0,
                        ..Default::default()
                    })
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        let _ = DestroyMenu(menu);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_resolves_lnk_and_url_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let program = Target::Program {
            path: "C:\\Windows\\System32\\notepad.exe".into(),
            arguments: "\"C:\\资料 文件.txt\"".into(),
            working_dir: "C:\\Windows".into(),
            elevated: true,
            show: ShowMode::Maximized,
        };
        let lnk = create_shortcut_in(dir.path(), "记事本", &program).unwrap();
        assert_eq!(resolve_lnk_file(&lnk), Some(program));

        let url = create_shortcut_in(
            dir.path(),
            "示例",
            &Target::Url {
                url: "https://example.com/a?q=1".into(),
            },
        )
        .unwrap();
        assert_eq!(
            resolve_url_file(&url).as_deref(),
            Some("https://example.com/a?q=1")
        );
    }
}
