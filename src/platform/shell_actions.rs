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

pub fn desktop_shortcut(name: &str, target: &Target) -> Result<std::path::PathBuf, String> {
    let _com = ComGuard::new()?;
    let desktop = unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)
            .map_err(|e| e.to_string())?;
        let value = p.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(p.0.cast()));
        std::path::PathBuf::from(value?)
    };
    create_shortcut_in(&desktop, name, target)
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
        safe = "Diascia 快捷方式".into();
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
        .unwrap_or_else(|| Err("同名快捷方式过多".into()))?;
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
    let hwnd = native_handle(window).ok_or("窗口未就绪")?;
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
                return Err("无法连接系统菜单".into());
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
    fn shortcuts_preserve_arguments_elevation_show_and_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = Target::Program {
            path: "C:\\Windows\\System32\\notepad.exe".into(),
            arguments: "\"C:\\资料 文件.txt\"".into(),
            working_dir: "C:\\Windows".into(),
            elevated: true,
            show: ShowMode::Maximized,
        };
        let first = create_shortcut_in(dir.path(), "测试", &target).unwrap();
        let bytes = std::fs::read(&first).unwrap();
        let second = create_shortcut_in(dir.path(), "测试", &target).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), bytes);
        let _com = ComGuard::new().unwrap();
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.cast::<IPersistFile>()
                .unwrap()
                .Load(PCWSTR(wide(&first.to_string_lossy()).as_ptr()), STGM_READ)
                .unwrap();
            let mut args = [0u16; 256];
            link.GetArguments(&mut args).unwrap();
            assert_eq!(
                String::from_utf16_lossy(&args[..args.iter().position(|c| *c == 0).unwrap()]),
                "\"C:\\资料 文件.txt\""
            );
            assert_ne!(
                link.cast::<IShellLinkDataList>()
                    .unwrap()
                    .GetFlags()
                    .unwrap()
                    & SLDF_RUNAS_USER.0 as u32,
                0
            );
            assert_eq!(link.GetShowCmd().unwrap(), show_cmd(ShowMode::Maximized));
        }
        let url = create_shortcut_in(
            dir.path(),
            "CON",
            &Target::Url {
                url: "https://example.com/a?q=1".into(),
            },
        )
        .unwrap();
        assert_eq!(url.file_name().unwrap(), "_CON.url");
        assert!(
            std::fs::read_to_string(url)
                .unwrap()
                .contains("URL=https://example.com/a?q=1")
        );
    }
    /// An 应用标识 must reach the writer and fail *inside* the Shell, not before it. A fake AUMID is
    /// deliberate: it makes `SHParseDisplayName` fail on every machine, so the assertion pins the
    /// failure's origin rather than the inventory of whatever is installed. The launcher used to
    /// absolutise through a convert that answers "网址和应用标识没有文件路径" for exactly this kind,
    /// so nothing here was ever reached.
    #[test]
    fn app_targets_reach_the_shell_instead_of_the_path_converter() {
        let dir = tempfile::tempdir().unwrap();
        let error = create_shortcut_in(
            dir.path(),
            "计算器",
            &Target::App {
                aumid: "Diascia.Probe_1.0.0.0_neutral__000000000000!Entry".into(),
            },
        )
        .expect_err("an uninstalled AUMID cannot resolve");
        assert!(!error.contains("文件路径"), "{error}");
        assert!(
            !dir.path().join("计算器.lnk").exists(),
            "a refused shortcut must not leave a file behind"
        );
    }
    /// Measured rather than assumed: the child list `SHBindToParent` reports is an **interior pointer
    /// into** the full list — `pidl + 440` on every run here — not an allocation of its own. Freeing
    /// it the way the Shell documentation suggests aborts the process with `STATUS_HEAP_CORRUPTION`,
    /// so this pins the shape: if a future Windows ever hands back a child from outside the block,
    /// this test fails and the ownership question has to be revisited deliberately.
    #[test]
    fn the_child_id_list_lives_inside_the_full_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool.exe");
        std::fs::write(&file, b"").unwrap();
        let _com = ComGuard::new().unwrap();
        unsafe {
            let mut full = std::ptr::null_mut();
            SHParseDisplayName(
                PCWSTR(wide(&file.to_string_lossy()).as_ptr()),
                None,
                &mut full,
                0,
                None,
            )
            .unwrap();
            let full = Pidl(full);
            let mut child = std::ptr::null_mut();
            let folder: IShellFolder =
                SHBindToParent(full.0, Some(&mut child)).expect("a real file has a parent folder");
            assert!(
                child > full.0 && (child as usize) - (full.0 as usize) < 4096,
                "child {child:p} 落在 full {:p} 之外，那它就该由调用方释放了",
                full.0
            );
            drop(folder);
        }
    }
}
