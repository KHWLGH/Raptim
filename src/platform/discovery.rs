//! Start Menu shortcut discovery, packaged AppsFolder enumeration and Shell icon extraction.
use super::*;
use shell_actions::known_folder_path;
use windows::core::Interface;

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
        let mut display_buf = Vec::new();
        let mut parsing_buf = Vec::new();
        let mut buffer = [std::ptr::null_mut::<Common::ITEMIDLIST>(); 1];
        while enumerator.Next(&mut buffer, None) == S_OK {
            let child = buffer[0];
            if child.is_null() {
                continue;
            }
            let display = child_name(&folder, child, SHGDN_NORMAL, 512, &mut display_buf);
            let parsing = child_name(&folder, child, SHGDN_FORPARSING, 4096, &mut parsing_buf);
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
    wide: &mut Vec<u16>,
) -> String {
    wide.resize(width, 0);
    let mut str_ret = Common::STRRET::default();
    if unsafe { folder.GetDisplayNameOf(child, flag, &mut str_ret) }.is_err()
        || unsafe { StrRetToBufW(&mut str_ret, Some(child), wide) }.is_err()
    {
        return String::new();
    }
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

/// Expand `%VAR%` environment placeholders commonly returned by `.lnk` icon locations.
fn expand_env_vars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            out.push_str(after);
            return out;
        };
        let name = &after[..end];
        if !name.is_empty()
            && !name.contains(['\\', '/'])
            && let Ok(val) = std::env::var(name)
        {
            out.push_str(&val);
        } else {
            out.push('%');
            out.push_str(name);
            out.push('%');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn resolve_icon_location(raw_path: &str) -> Option<std::path::PathBuf> {
    let expanded = expand_env_vars(raw_path.trim().trim_matches('"').trim());
    if expanded.is_empty() {
        return None;
    }
    let direct = Path::new(&expanded);
    if direct.is_file() {
        return Some(direct.to_path_buf());
    }
    if !expanded.contains('\\') && !expanded.contains('/') {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| String::from(r"C:\Windows"));
        let sys32 = Path::new(&root).join("System32").join(&expanded);
        if sys32.is_file() {
            return Some(sys32);
        }
    }
    None
}

fn lnk_icon_candidates(path: &Path) -> Vec<(std::path::PathBuf, i32)> {
    if !path.is_file() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    unsafe {
        let Ok(link): windows::core::Result<IShellLinkW> =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
        else {
            return candidates;
        };
        let Ok(persist): windows::core::Result<IPersistFile> = link.cast() else {
            return candidates;
        };
        let wide_path = wide(&path.to_string_lossy());
        if persist.Load(PCWSTR(wide_path.as_ptr()), STGM_READ).is_err() {
            return candidates;
        }
        let mut icon_buf = [0u16; 32768];
        let mut icon_index = 0i32;
        if link.GetIconLocation(&mut icon_buf, &mut icon_index).is_ok() {
            let end = icon_buf
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(icon_buf.len());
            let raw = String::from_utf16_lossy(&icon_buf[..end]);
            if let Some(resolved) = resolve_icon_location(&raw) {
                candidates.push((resolved, icon_index));
            }
        }
        let darwin = link
            .cast::<IShellLinkDataList>()
            .ok()
            .and_then(|data| data.GetFlags().ok())
            .is_some_and(|flags| flags & SLDF_HAS_DARWINID.0 as u32 != 0);
        if !darwin {
            let mut target_buf = [0u16; 32768];
            if link
                .GetPath(&mut target_buf, std::ptr::null_mut(), 0)
                .is_ok()
            {
                let end = target_buf
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(target_buf.len());
                let raw = String::from_utf16_lossy(&target_buf[..end]);
                if let Some(resolved) = resolve_icon_location(&raw)
                    && candidates.iter().all(|(p, i)| p != &resolved || *i != 0)
                {
                    candidates.push((resolved, 0));
                }
            }
        }
    }
    candidates
}

fn extract_direct_resource_icon(path: &Path, icon_index: i32) -> Option<image::RgbaImage> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    let img = match ext.as_str() {
        "ico" => {
            let meta = std::fs::metadata(path).ok()?;
            if !meta.is_file() || meta.len() > 16 * 1024 * 1024 {
                return None;
            }
            let bytes = std::fs::read(path).ok()?;
            crate::icons::decode_best_ico(&bytes)?
        }
        "exe" | "dll" | "cpl" | "scr" | "ocx" | "mun" => {
            crate::icons::extract_pe_icon(path, icon_index)?
        }
        _ => return None,
    };
    img.pixels().any(|p| p[3] > 0).then_some(img)
}

struct OwnedBitmap(HBITMAP);
impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

struct OwnedDc(HDC);
impl Drop for OwnedDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

unsafe fn hbitmap_to_rgba(bitmap: HBITMAP) -> Option<image::RgbaImage> {
    if bitmap.is_invalid() {
        return None;
    }
    let mut bm = BITMAP::default();
    if unsafe {
        GetObjectW(
            bitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut bm as *mut BITMAP).cast()),
        )
    } == 0
    {
        return None;
    }
    let width = u32::try_from(bm.bmWidth).ok()?;
    let height = bm.bmHeight.unsigned_abs();
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let dc = OwnedDc(unsafe { CreateCompatibleDC(None) });
    if dc.0.is_invalid() {
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let byte_len = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    let mut bytes = vec![0u8; byte_len];
    let copied = unsafe {
        GetDIBits(
            dc.0,
            bitmap,
            0,
            height,
            Some(bytes.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    if copied == 0 || bytes.iter().all(|&b| b == 0) {
        return None;
    }
    let all_alpha_zero = bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    for p in bytes.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        if all_alpha_zero {
            p[3] = 255;
        } else if p[3] > 0 && p[3] < 255 {
            let a = u32::from(p[3]);
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    let image = image::RgbaImage::from_raw(width, height, bytes)?;
    let trimmed = crate::icons::trim_jumbo_corner_padding(image);
    trimmed.pixels().any(|p| p[3] > 0).then_some(trimmed)
}

unsafe fn shell_item_image(
    parsing_name: &str,
    allow_thumbnail_fallback: bool,
) -> Option<image::RgbaImage> {
    let wide_name = wide(parsing_name);
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide_name.as_ptr()), None) }.ok()?;
    let size = SIZE { cx: 256, cy: 256 };
    for flag in [SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY] {
        if let Ok(bmp) = unsafe { factory.GetImage(size, flag) } {
            let guard = OwnedBitmap(bmp);
            if let Some(image) = unsafe { hbitmap_to_rgba(guard.0) } {
                return Some(image);
            }
        }
    }
    if allow_thumbnail_fallback
        && let Ok(bmp) = unsafe { factory.GetImage(size, SIIGBF_BIGGERSIZEOK) }
    {
        let guard = OwnedBitmap(bmp);
        return unsafe { hbitmap_to_rgba(guard.0) };
    }
    None
}

/// Extract the highest-resolution icon via direct resource inspection, Shell item image factory,
/// and `SHGetFileInfoW` fallback, then save Lanczos3-downscaled PNG pixels.
pub fn shell_icon(path: &Path) -> Vec<u8> {
    let is_lnk = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
    if !is_lnk
        && let Some(image) = extract_direct_resource_icon(path, 0)
        && let Ok(png) = crate::icons::encode(image)
    {
        return png;
    }
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    if is_lnk
        && let Some(image) = lnk_icon_candidates(path)
            .into_iter()
            .find_map(|(candidate, index)| extract_direct_resource_icon(&candidate, index))
        && let Ok(png) = crate::icons::encode(image)
    {
        return png;
    }

    let normalized = path.to_string_lossy().replace('/', "\\");
    if let Some(image) = unsafe { shell_item_image(&normalized, false) }
        && let Ok(png) = crate::icons::encode(image)
    {
        return png;
    }

    unsafe {
        let mut info = SHFILEINFOW::default();
        let name = wide(&normalized);
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

/// Packaged applications have no file system path: request their high-resolution logo through
/// `IShellItemImageFactory` first, falling back to the AppsFolder Shell image list.
fn app_icon(aumid: &str) -> Vec<u8> {
    if !aumid.contains('!') || aumid.contains('\\') {
        return Vec::new();
    }
    let Ok(_com) = ComGuard::new() else {
        return vec![];
    };
    let parsing = format!("shell:AppsFolder\\{aumid}");
    if let Some(image) = unsafe { shell_item_image(&parsing, true) }
        && let Ok(png) = crate::icons::encode(image)
    {
        return png;
    }
    unsafe {
        let name = wide(&parsing);
        let mut id: *mut Common::ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(PCWSTR(name.as_ptr()), None, &mut id, 0, None).is_err() {
            return Vec::new();
        }
        let _item = super::Pidl(id);
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
            .map(crate::icons::trim_jumbo_corner_padding)
            .and_then(|i| crate::icons::encode(i).ok())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
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
    let _com = ComGuard::new().ok();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_environment_variables_in_icon_locations() {
        let expanded = expand_env_vars(r"%SystemRoot%\System32\shell32.dll");
        if let Ok(sys_root) = std::env::var("SystemRoot") {
            assert_eq!(expanded, format!(r"{sys_root}\System32\shell32.dll"));
        }
        assert_eq!(
            expand_env_vars(r"C:\Tools\%RAPTIM_MISSING_TEST_VAR%\app.exe"),
            r"C:\Tools\%RAPTIM_MISSING_TEST_VAR%\app.exe"
        );
        assert_eq!(expand_env_vars("plain%unclosed"), "plain%unclosed");
    }

    #[test]
    fn shell_icon_extracts_high_res_icon_from_system_exe() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.is_file() {
            return;
        }
        let png = shell_icon(notepad);
        assert!(!png.is_empty());
        let decoded = image::load_from_memory(&png).unwrap();
        assert!(
            decoded.width().min(decoded.height()) >= 48,
            "expected >=48px icon, got {}x{}",
            decoded.width(),
            decoded.height()
        );
    }
}
