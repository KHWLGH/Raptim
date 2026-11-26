//! Start Menu shortcut discovery, packaged AppsFolder enumeration and Shell icon extraction.
use super::*;

/// AppsFolder gives one display name plus one parsing name per child; only entries that parse to
/// an app id are usable as a launch target, so everything else (virtual sub-folders in
/// particular) is dropped here rather than becoming a broken project.
pub(super) fn app_option(display: &str, parsing: &str) -> Option<(String, String)> {
    // The parent folder hands back a child-relative parsing name, which is the bare app id; the
    // absolute form is accepted too because that is what a dropped Shell item reports.
    let aumid = parsing
        .strip_prefix("shell:AppsFolder\\")
        .unwrap_or(parsing)
        .trim_end_matches('\\');
    if !aumid.contains('!') || aumid.contains('\\') || display.trim().is_empty() {
        return None;
    }
    Some((aumid.to_string(), display.to_string()))
}

/// Installed packaged applications as `(aumid, display name)`.
pub fn installed_apps() -> Vec<(String, String)> {
    // Best effort: a thread that already initialised a different apartment model still resolves
    // shell items, and a thread with no COM at all simply gets an empty list back.
    let _com = ComGuard::new().ok();
    unsafe {
        let Ok(item): windows::core::Result<IShellItem> =
            SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)
        else {
            return Vec::new();
        };
        let Ok(folder): windows::core::Result<IShellFolder> =
            item.BindToHandler(None, &BHID_SFObject)
        else {
            return Vec::new();
        };
        let mut enumerator: Option<IEnumIDList> = None;
        if folder
            .EnumObjects(
                HWND::default(),
                (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INIT_ON_FIRST_NEXT.0) as u32,
                &mut enumerator,
            )
            .is_err()
        {
            return Vec::new();
        }
        let Some(enumerator) = enumerator else {
            return Vec::new();
        };
        let mut apps = Vec::new();
        let mut buffer = [std::ptr::null_mut::<Common::ITEMIDLIST>(); 1];
        while enumerator.Next(&mut buffer, None) == S_OK {
            let child = buffer[0];
            if child.is_null() {
                continue;
            }
            let display = child_name(&folder, child, SHGDN_NORMAL, 512);
            let parsing = child_name(&folder, child, SHGDN_FORPARSING, 4096);
            CoTaskMemFree(Some(child as *const _));
            if let Some(option) = app_option(&display, &parsing) {
                apps.push(option);
            }
        }
        apps.sort_by_key(|app| app.1.to_lowercase());
        apps
    }
}

fn child_name(
    folder: &IShellFolder,
    child: *const Common::ITEMIDLIST,
    flag: SHGDNF,
    width: usize,
) -> String {
    let mut wide = vec![0u16; width];
    let mut str_ret = Common::STRRET::default();
    if unsafe { folder.GetDisplayNameOf(child, flag, &mut str_ret) }.is_err()
        || unsafe { StrRetToBufW(&mut str_ret, Some(child), &mut wide) }.is_err()
    {
        return String::new();
    }
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

/// Extract the Windows Shell icon (including shortcut resolution) and save PNG pixels.
pub fn shell_icon(path: &Path) -> Vec<u8> {
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    unsafe {
        let mut info = SHFILEINFOW::default();
        let name = wide(&path.to_string_lossy().replace('/', "\\"));
        if SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX,
        ) == 0
        {
            return vec![];
        }
        icon_by_index(info.iIcon)
    }
}

/// Packaged applications have no file system path: their icon is served by the AppsFolder
/// namespace, so the display name is parsed into an item id and looked up in the same image list.
pub fn app_icon(aumid: &str) -> Vec<u8> {
    if !aumid.contains('!') || aumid.contains('\\') {
        return Vec::new();
    }
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    unsafe {
        let name = wide(&format!("shell:AppsFolder\\{aumid}"));
        let mut id: *mut Common::ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut id, 0, None).is_err() {
            return Vec::new();
        }
        let _item = TaskMemIdl(id);
        let mut info = SHFILEINFOW::default();
        if SHGetFileInfoW(
            PCWSTR(id as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_SYSICONINDEX,
        ) == 0
        {
            return Vec::new();
        }
        icon_by_index(info.iIcon)
    }
}

/// Single dispatch used by every icon call site: a web page is asked for its own icon, a packaged
/// app is resolved through the Shell namespace, and everything else is a path on disk.
pub fn icon_for(target: &Target, base: &Path) -> Vec<u8> {
    match target {
        Target::Url { url } => web_icon::fetch(url),
        Target::App { aumid } => app_icon(aumid),
        other => shell_icon(&crate::paths::absolute(other.value(), base)),
    }
}

/// Draw one system image-list entry onto a transparent square and encode it as PNG.
unsafe fn icon_by_index(index: i32) -> Vec<u8> {
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    unsafe {
        let (icon, size) = [
            (SHIL_JUMBO as i32, 256),
            (SHIL_EXTRALARGE as i32, 48),
            (SHIL_LARGE as i32, 32),
        ]
        .into_iter()
        .find_map(|(list, size)| {
            SHGetImageList::<IImageList>(list)
                .ok()
                .and_then(|images| images.GetIcon(index, ILD_TRANSPARENT.0).ok())
                .map(|icon| (icon, size))
        })
        .unwrap_or((
            windows::Win32::UI::WindowsAndMessaging::HICON::default(),
            32,
        ));
        if icon.is_invalid() {
            return vec![];
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            let _ = DestroyIcon(icon);
            return vec![];
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            let _ = DestroyIcon(icon);
            return vec![];
        };
        let old = SelectObject(dc, bitmap.into());
        std::ptr::write_bytes(bits, 0, (size * size * 4) as usize);
        let result = DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL);
        let mut bytes =
            std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize).to_vec();
        // Legacy mask-based icons have no alpha. Drawing on white and black
        // recovers coverage, including opaque black pixels (not a color key).
        if bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            std::ptr::write_bytes(bits, 255, (size * size * 4) as usize);
            if DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL).is_ok() {
                let white =
                    std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize);
                for (p, w) in bytes
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(white.as_chunks::<4>().0)
                {
                    p[3] = 255 - w[0].saturating_sub(p[0]);
                }
            }
        }
        let _ = SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        let _ = DestroyIcon(icon);
        if result.is_err() {
            return vec![];
        }
        for p in bytes.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 / a).min(255)) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(size as u32, size as u32, bytes)
            .and_then(|i| crate::icons::encode(i).ok())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredApp {
    pub name: String,
    pub detail: String,
    pub target: Target,
    pub icon_source: Option<std::path::PathBuf>,
}

fn is_noise_shortcut(name: &str, target: &Target) -> bool {
    let low_name = name.to_lowercase();
    let low_val = target.value().to_lowercase();
    [
        "uninstall",
        "uninst",
        "卸载",
        "readme",
        "读我",
        "帮助",
        "manual",
        "license",
        "用户协议",
    ]
    .iter()
    .any(|kw| low_name.contains(kw))
        || low_val.contains("unins0")
        || low_val.contains("uninstall")
}

fn collect_shortcuts_in(
    dir: &Path,
    depth: u8,
    self_exe: &str,
    seen_keys: &mut std::collections::HashSet<String>,
    out: &mut Vec<DiscoveredApp>,
) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() && !ft.is_symlink() {
            collect_shortcuts_in(&path, depth + 1, self_exe, seen_keys, out);
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "lnk" | "url") {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let target = if ext == "url" {
            match resolve_url_file(&path) {
                Some(url) => Target::Url { url },
                None => Target::Shortcut {
                    path: path.to_string_lossy().into_owned(),
                    show: ShowMode::Normal,
                },
            }
        } else {
            resolve_lnk_file(&path).unwrap_or_else(|| Target::Shortcut {
                path: path.to_string_lossy().into_owned(),
                show: ShowMode::Normal,
            })
        };
        if target.validate().is_err() || is_noise_shortcut(&name, &target) {
            continue;
        }
        if !self_exe.is_empty()
            && target
                .value()
                .replace('/', "\\")
                .eq_ignore_ascii_case(self_exe)
        {
            continue;
        }
        if seen_keys.insert(target.duplicate_key()) {
            let detail = target.value().to_string();
            out.push(DiscoveredApp {
                name,
                detail,
                target,
                icon_source: Some(path),
            });
        }
    }
}

/// Discover installed applications from Start Menu shortcuts and packaged AppsFolder entries.
pub fn discover_installed_entries() -> Vec<DiscoveredApp> {
    let self_exe = std::env::current_exe()
        .map(|p| {
            crate::paths::normal_path(&p)
                .to_string_lossy()
                .replace('/', "\\")
        })
        .unwrap_or_default();
    let mut seen_keys = std::collections::HashSet::new();
    let mut out = Vec::new();
    for folder in [&FOLDERID_CommonPrograms, &FOLDERID_Programs] {
        if let Ok(dir) = known_folder_path(folder) {
            collect_shortcuts_in(&dir, 0, &self_exe, &mut seen_keys, &mut out);
        }
    }
    let seen_names: std::collections::HashSet<String> =
        out.iter().map(|d| d.name.to_lowercase()).collect();
    for (aumid, name) in installed_apps() {
        let target = Target::App {
            aumid: aumid.clone(),
        };
        if seen_names.contains(&name.to_lowercase()) || target.validate().is_err() {
            continue;
        }
        if seen_keys.insert(target.duplicate_key()) {
            out.push(DiscoveredApp {
                name,
                detail: aumid,
                target,
                icon_source: None,
            });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}
