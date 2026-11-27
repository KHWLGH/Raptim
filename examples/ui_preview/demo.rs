use raptim::{
    AppOption, BrokenOption, DestinationNode, DiscoverOption, Item, NavItem, NoticeItem, ScopeNode,
    model::{Collection, Entry, Export, Target},
};
use std::path::Path;

pub struct PreviewData {
    pub groups: Vec<NavItem>,
    pub categories: Vec<NavItem>,
    pub active_group: i32,
    pub active_category: i32,
    pub grid_items: Vec<Item>,
    pub list_items: Vec<Item>,
    pub search_query: String,
    pub search_results: Vec<Item>,
    pub batch_nodes: Vec<DestinationNode>,
    pub discover_items: Vec<DiscoverOption>,
    pub broken_items: Vec<BrokenOption>,
    pub conversion_scopes: Vec<ScopeNode>,
    pub picker_apps: Vec<AppOption>,
    pub protected_apps: Vec<String>,
    pub data_path: String,
    pub license_lines: Vec<String>,
    pub notice_items: Vec<NoticeItem>,
    pub editor_program_icon: slint::Image,
    pub editor_url_icon: slint::Image,
}

#[derive(Clone, Copy)]
enum Glyph {
    CodeBrackets,
    Terminal,
    Magnifier,
    Crystal,
    ChatBubbles,
    Folder,
    Document,
    BranchMark,
    Globe,
    StoreBag,
    Palette,
    Calculator,
}

fn sdf_box(px: f32, py: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = px.abs() - half_w + r;
    let qy = py.abs() - half_h + r;
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

fn sdf_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, thickness: f32) -> f32 {
    let pax = px - ax;
    let pay = py - ay;
    let bax = bx - ax;
    let bay = by - ay;
    let denom = bax * bax + bay * bay;
    let h = if denom > 0.0 {
        ((pax * bax + pay * bay) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    (dx * dx + dy * dy).sqrt() - thickness
}

fn sdf_ring(px: f32, py: f32, cx: f32, cy: f32, r: f32, thickness: f32) -> f32 {
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r;
    d.abs() - thickness
}

fn glyph_sdf(glyph: Glyph, u: f32, v: f32) -> f32 {
    match glyph {
        Glyph::CodeBrackets => {
            let l1 = sdf_segment(u, v, -0.10, -0.20, -0.26, 0.0, 0.045);
            let l2 = sdf_segment(u, v, -0.26, 0.0, -0.10, 0.20, 0.045);
            let r1 = sdf_segment(u, v, 0.10, -0.20, 0.26, 0.0, 0.045);
            let r2 = sdf_segment(u, v, 0.26, 0.0, 0.10, 0.20, 0.045);
            let slash = sdf_segment(u, v, 0.04, -0.23, -0.04, 0.23, 0.036);
            l1.min(l2).min(r1).min(r2).min(slash)
        }
        Glyph::Terminal => {
            let c1 = sdf_segment(u, v, -0.22, -0.14, -0.05, 0.0, 0.046);
            let c2 = sdf_segment(u, v, -0.05, 0.0, -0.22, 0.14, 0.046);
            let line = sdf_segment(u, v, 0.04, 0.15, 0.24, 0.15, 0.045);
            c1.min(c2).min(line)
        }
        Glyph::Magnifier => {
            let ring = sdf_ring(u, v, -0.05, -0.05, 0.16, 0.042);
            let handle = sdf_segment(u, v, 0.07, 0.07, 0.24, 0.24, 0.050);
            ring.min(handle)
        }
        Glyph::Crystal => {
            let d = (u.abs() * 1.25 + v.abs() * 0.9) - 0.28;
            let facet = sdf_segment(u, v, 0.0, -0.26, 0.0, 0.26, 0.025);
            d.max(-facet * 0.6)
        }
        Glyph::ChatBubbles => {
            let b1 = ((u + 0.08).powi(2) / 0.22f32.powi(2) + (v + 0.05).powi(2) / 0.16f32.powi(2))
                .sqrt()
                - 1.0;
            let b2 = ((u - 0.12).powi(2) / 0.17f32.powi(2) + (v - 0.09).powi(2) / 0.13f32.powi(2))
                .sqrt()
                - 1.0;
            (b1 * 0.16).min(b2 * 0.14)
        }
        Glyph::Folder => {
            let body = sdf_box(u, v - 0.03, 0.27, 0.18, 0.045);
            let tab = sdf_box(u + 0.11, v + 0.14, 0.15, 0.07, 0.03);
            body.min(tab)
        }
        Glyph::Document => {
            let page = sdf_box(u, v, 0.20, 0.25, 0.04);
            let line1 = sdf_segment(u, v, -0.10, -0.09, 0.10, -0.09, 0.025);
            let line2 = sdf_segment(u, v, -0.10, 0.01, 0.10, 0.01, 0.025);
            let line3 = sdf_segment(u, v, -0.10, 0.11, 0.04, 0.11, 0.025);
            let cutout = line1.min(line2).min(line3);
            page.max(-cutout)
        }
        Glyph::BranchMark => {
            let stem = sdf_segment(u, v, -0.12, -0.20, -0.12, 0.20, 0.038);
            let branch = sdf_segment(u, v, -0.12, 0.04, 0.12, -0.10, 0.036);
            let n1 = ((u + 0.12).powi(2) + (v + 0.20).powi(2)).sqrt() - 0.075;
            let n2 = ((u + 0.12).powi(2) + (v - 0.20).powi(2)).sqrt() - 0.075;
            let n3 = ((u - 0.12).powi(2) + (v + 0.10).powi(2)).sqrt() - 0.075;
            stem.min(branch).min(n1).min(n2).min(n3)
        }
        Glyph::Globe => {
            let outer = sdf_ring(u, v, 0.0, 0.0, 0.23, 0.032);
            let horiz = sdf_segment(u, v, -0.22, 0.0, 0.22, 0.0, 0.026);
            let vert = sdf_segment(u, v, 0.0, -0.22, 0.0, 0.22, 0.026);
            let merid = ((u / 0.11).powi(2) + (v / 0.23).powi(2)).sqrt() - 1.0;
            outer.min(horiz).min(vert).min(merid.abs() * 0.12 - 0.024)
        }
        Glyph::StoreBag => {
            let bag = sdf_box(u, v - 0.05, 0.22, 0.18, 0.04);
            let handle = sdf_ring(u, v + 0.13, 0.0, 0.0, 0.10, 0.028).max(v + 0.13);
            let grid = sdf_box(u, v - 0.05, 0.09, 0.09, 0.01);
            bag.min(handle).max(-grid.min(0.0) * 0.0)
        }
        Glyph::Palette => {
            let ring = sdf_ring(u, v, 0.0, 0.0, 0.20, 0.045);
            let dot1 = ((u - 0.07).powi(2) + (v + 0.07).powi(2)).sqrt() - 0.055;
            let dot2 = ((u + 0.08).powi(2) + (v - 0.06).powi(2)).sqrt() - 0.055;
            ring.min(dot1).min(dot2)
        }
        Glyph::Calculator => {
            let d1 = sdf_box(u + 0.11, v + 0.11, 0.075, 0.075, 0.025);
            let d2 = sdf_box(u - 0.11, v + 0.11, 0.075, 0.075, 0.025);
            let d3 = sdf_box(u + 0.11, v - 0.11, 0.075, 0.075, 0.025);
            let d4 = sdf_box(u - 0.11, v - 0.11, 0.075, 0.075, 0.025);
            d1.min(d2).min(d3).min(d4)
        }
    }
}

fn make_badge_icon(top: [u8; 3], bottom: [u8; 3], glyph: Glyph) -> slint::Image {
    let size = 96u32;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let half = size as f32 * 0.5;
    let badge_half = 44.0f32;
    let badge_r = 14.0f32;
    let sub_offsets = [-0.375f32, -0.125, 0.125, 0.375];

    for y in 0..size {
        for x in 0..size {
            let mut sum_r = 0.0f32;
            let mut sum_g = 0.0f32;
            let mut sum_b = 0.0f32;
            let mut sum_a = 0.0f32;

            for &sy in &sub_offsets {
                for &sx in &sub_offsets {
                    let px = x as f32 + 0.5 + sx - half;
                    let py = y as f32 + 0.5 + sy - half;
                    let bg_dist = sdf_box(px, py, badge_half, badge_half, badge_r);
                    let bg_cov = if bg_dist <= 0.0 { 1.0 } else { 0.0 };
                    if bg_cov == 0.0 {
                        continue;
                    }
                    let t = ((y as f32 + 0.5 + sy) / size as f32).clamp(0.0, 1.0);
                    let mut r = f32::from(top[0]) + (f32::from(bottom[0]) - f32::from(top[0])) * t;
                    let mut g = f32::from(top[1]) + (f32::from(bottom[1]) - f32::from(top[1])) * t;
                    let mut b = f32::from(top[2]) + (f32::from(bottom[2]) - f32::from(top[2])) * t;

                    // Subtle top highlight for depth.
                    if bg_dist > -2.5 {
                        r = (r + 18.0).min(255.0);
                        g = (g + 18.0).min(255.0);
                        b = (b + 18.0).min(255.0);
                    }

                    let u = px / (badge_half * 2.0);
                    let v = py / (badge_half * 2.0);
                    if glyph_sdf(glyph, u, v) <= 0.0 {
                        r = 250.0;
                        g = 252.0;
                        b = 255.0;
                    }

                    sum_r += r;
                    sum_g += g;
                    sum_b += b;
                    sum_a += 1.0;
                }
            }

            let count = (sub_offsets.len() * sub_offsets.len()) as f32;
            let idx = ((y * size + x) * 4) as usize;
            if sum_a > 0.0 {
                pixels[idx] = (sum_r / sum_a).round() as u8;
                pixels[idx + 1] = (sum_g / sum_a).round() as u8;
                pixels[idx + 2] = (sum_b / sum_a).round() as u8;
                pixels[idx + 3] = ((sum_a / count) * 255.0).round() as u8;
            }
        }
    }

    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &pixels, size, size,
    ))
}

fn nav(id: i32, name: &str) -> NavItem {
    NavItem {
        id,
        name: name.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn make_item(
    id: i32,
    name: &str,
    detail: &str,
    tooltip_path: &str,
    tooltip_notes: &str,
    kind: i32,
    icon: slint::Image,
    checked: bool,
) -> Item {
    Item {
        checked,
        id,
        name: name.into(),
        detail: detail.into(),
        tooltip_path: tooltip_path.into(),
        tooltip_notes: tooltip_notes.into(),
        kind,
        drag_icon: icon.clone(),
        icon,
    }
}

fn default_license_lines() -> Vec<String> {
    include_str!("../../LICENSE")
        .lines()
        .map(str::to_string)
        .collect()
}

fn default_notice_items(en: bool) -> Vec<NoticeItem> {
    let t = |zh, en_s| if en { en_s } else { zh };
    [
        (
            "slint / i-slint-core / slint-build",
            "1.18.0",
            "GPL-3.0-only",
            t(
                "界面框架与 Fluent 软件渲染后端",
                "UI framework and Fluent software renderer",
            ),
        ),
        (
            "rusqlite",
            "0.37.0",
            "MIT",
            t(
                "SQLite 存储封装，内置 SQLite 源码编译",
                "SQLite wrapper with bundled SQLite source",
            ),
        ),
        (
            "windows / windows-core",
            "0.62.2",
            "MIT OR Apache-2.0",
            t(
                "Win32 窗口、Shell、拖放、热键与网络接口",
                "Win32 windowing, Shell, drag-and-drop, hotkeys, and HTTP bindings",
            ),
        ),
        (
            "tray-icon",
            "0.24.2",
            "MIT OR Apache-2.0",
            t("系统托盘图标与托盘菜单", "System tray icon and menu"),
        ),
        (
            "rfd",
            "0.16.0",
            "MIT",
            t(
                "系统原生文件选择、目录选择与提示框",
                "Native file, folder, and message dialogs",
            ),
        ),
        (
            "image",
            "0.25.10",
            "MIT OR Apache-2.0",
            t(
                "PNG、JPEG 与 ICO 图标的解码、缩放与编码",
                "PNG, JPEG, and ICO image decoding, scaling, and encoding",
            ),
        ),
        (
            "pinyin",
            "0.10.0",
            "MIT",
            t(
                "离线中文全拼与首字母搜索",
                "Offline Chinese Pinyin and initial-letter search",
            ),
        ),
        (
            "arboard",
            "3.6.1",
            "MIT OR Apache-2.0",
            t("系统剪贴板读写", "System clipboard read/write"),
        ),
        (
            "base64",
            "0.23.1",
            "MIT OR Apache-2.0",
            t(
                "JSON 导入导出时的图标 Base64 编解码",
                "Icon Base64 encoding and decoding for JSON export/restore",
            ),
        ),
        (
            "serde / serde_json",
            "1.0.229 / 1.0.151",
            "MIT OR Apache-2.0",
            t(
                "配置、本机窗口状态与 JSON 导入导出的序列化",
                "Serialization for settings, window state, and JSON export/restore",
            ),
        ),
        (
            "url",
            "2.5.8",
            "MIT OR Apache-2.0",
            t(
                "网址解析与网页图标地址拼接",
                "URL parsing and favicon URL resolution",
            ),
        ),
        (
            "thiserror",
            "2.0.20",
            "MIT OR Apache-2.0",
            t("错误类型定义", "Error type derivation"),
        ),
        (
            "raw-window-handle",
            "0.6.2",
            "MIT OR Apache-2.0 OR Zlib",
            t(
                "获取窗口底层 Win32 HWND 句柄",
                "Raw Win32 HWND handle access",
            ),
        ),
    ]
    .into_iter()
    .map(|(name, version, license, detail)| NoticeItem {
        name: name.into(),
        version: version.into(),
        license: license.into(),
        detail: detail.into(),
    })
    .collect()
}

fn default_discover_items(en: bool) -> Vec<DiscoverOption> {
    let t = |zh, en_s| if en { en_s } else { zh };
    [
        (
            true,
            0,
            "Visual Studio Code",
            r"C:\Program Files\Microsoft VS Code\Code.exe",
            0,
        ),
        (
            true,
            1,
            t("Windows 终端", "Windows Terminal"),
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
            5,
        ),
        (
            false,
            2,
            "7-Zip File Manager",
            r"C:\Program Files\7-Zip\7zFM.exe",
            0,
        ),
        (
            true,
            3,
            "PowerToys",
            r"C:\Program Files\PowerToys\PowerToys.exe",
            0,
        ),
        (
            false,
            4,
            "OBS Studio",
            r"C:\Program Files\obs-studio\bin\64bit\obs64.exe",
            0,
        ),
        (false, 5, "Notion", r"D:\Apps\Notion\Notion.exe", 0),
        (
            false,
            6,
            "Spotify",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            5,
        ),
    ]
    .into_iter()
    .map(|(checked, index, name, detail, _kind)| DiscoverOption {
        checked,
        index,
        name: name.into(),
        detail: detail.into(),
    })
    .collect()
}

fn default_broken_items(en: bool) -> Vec<BrokenOption> {
    let t = |zh, en_s| if en { en_s } else { zh };
    [
        (
            901,
            t("旧版 Figma", "Legacy Figma"),
            r"D:\Apps\Figma\Figma.exe",
            t(
                "目标文件或快捷方式不存在",
                "Target file or shortcut does not exist",
            ),
            true,
        ),
        (
            902,
            t("项目归档目录", "Project Archive"),
            r"E:\Archive\2025-Q4",
            t("目标文件夹不存在", "Target folder does not exist"),
            true,
        ),
        (
            903,
            t("测试工具 UWP", "Preview Tool UWP"),
            "Contoso.DevPreview_1.0!App",
            t("未找到对应的已安装应用", "Installed app not found"),
            false,
        ),
    ]
    .into_iter()
    .map(|(id, name, target, reason, relocatable)| BrokenOption {
        id,
        name: name.into(),
        target: target.into(),
        reason: reason.into(),
        relocatable,
    })
    .collect()
}

fn default_picker_apps(en: bool) -> Vec<AppOption> {
    let t = |zh, en_s| if en { en_s } else { zh };
    [
        (
            t("Windows 终端", "Windows Terminal"),
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
        ),
        (
            "Microsoft Store",
            "Microsoft.WindowsStore_8wekyb3d8bbwe!App",
        ),
        (
            t("计算器", "Calculator"),
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
        ),
        (t("画图", "Paint"), "Microsoft.Paint_8wekyb3d8bbwe!App"),
        (
            t("截图工具", "Snipping Tool"),
            "Microsoft.ScreenSketch_8wekyb3d8bbwe!App",
        ),
        (
            t("记事本", "Notepad"),
            "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
        ),
    ]
    .into_iter()
    .map(|(name, aumid)| AppOption {
        name: name.into(),
        aumid: aumid.into(),
    })
    .collect()
}

/// Curated, privacy-safe demo dataset covering all six Raptim target kinds and overlays.
pub fn demo_data(en: bool) -> PreviewData {
    let t = |zh, en_s| if en { en_s } else { zh };
    let code_icon = make_badge_icon([29, 148, 240], [11, 95, 184], Glyph::CodeBrackets);
    let term_icon = make_badge_icon([52, 60, 74], [24, 28, 36], Glyph::Terminal);
    let search_icon = make_badge_icon([245, 166, 35], [217, 119, 6], Glyph::Magnifier);
    let notes_icon = make_badge_icon([139, 92, 246], [91, 33, 182], Glyph::Crystal);
    let chat_icon = make_badge_icon([34, 197, 94], [21, 128, 61], Glyph::ChatBubbles);
    let folder_icon = make_badge_icon([234, 179, 8], [180, 125, 10], Glyph::Folder);
    let doc_icon = make_badge_icon([59, 130, 246], [29, 78, 216], Glyph::Document);
    let github_icon = make_badge_icon([51, 65, 85], [15, 23, 42], Glyph::BranchMark);
    let globe_icon = make_badge_icon([20, 184, 166], [15, 118, 110], Glyph::Globe);
    let store_icon = make_badge_icon([14, 165, 233], [3, 105, 161], Glyph::StoreBag);
    let paint_icon = make_badge_icon([236, 72, 153], [190, 24, 93], Glyph::Palette);
    let calc_icon = make_badge_icon([100, 116, 139], [71, 85, 105], Glyph::Calculator);

    let grid_items = vec![
        make_item(
            1,
            "VS Code",
            r"C:\Program Files\Microsoft VS Code\Code.exe",
            r"C:\Program Files\Microsoft VS Code\Code.exe",
            t("主力代码编辑器", "Primary code editor"),
            0,
            code_icon.clone(),
            true,
        ),
        make_item(
            2,
            t("Windows 终端", "Windows Terminal"),
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
            t("PowerShell 与 WSL 终端", "PowerShell & WSL terminal"),
            5,
            term_icon.clone(),
            true,
        ),
        make_item(
            3,
            "Everything",
            r"C:\Program Files\Everything\Everything.exe",
            r"C:\Program Files\Everything\Everything.exe",
            t("本地文件秒级检索", "Instant local file search"),
            0,
            search_icon,
            false,
        ),
        make_item(
            4,
            "Obsidian",
            r"D:\Apps\Obsidian\Obsidian.exe",
            r"D:\Apps\Obsidian\Obsidian.exe",
            t(
                "个人知识库与 Markdown 笔记",
                "Knowledge base & Markdown notes",
            ),
            0,
            notes_icon,
            true,
        ),
        make_item(
            5,
            t("微信", "WeChat"),
            r"C:\Program Files\Tencent\WeChat\WeChat.exe",
            r"C:\Program Files\Tencent\WeChat\WeChat.exe",
            t("桌面即时通讯", "Desktop instant messaging"),
            0,
            chat_icon,
            false,
        ),
        make_item(
            6,
            t("项目工作区", "Projects"),
            r"D:\Workspace\Projects",
            r"D:\Workspace\Projects",
            t("本地源码仓库目录", "Local source code repositories"),
            2,
            folder_icon.clone(),
            false,
        ),
        make_item(
            7,
            t("产品方案.md", "Notes.md"),
            t(
                r"D:\Workspace\Docs\产品方案.md",
                r"D:\Workspace\Docs\Notes.md",
            ),
            t(
                r"D:\Workspace\Docs\产品方案.md",
                r"D:\Workspace\Docs\Notes.md",
            ),
            t("版本迭代与需求规划", "Release planning and feature specs"),
            1,
            doc_icon,
            false,
        ),
        make_item(
            8,
            "GitHub",
            "https://github.com",
            "https://github.com",
            t("开源仓库与 Issue 跟踪", "Open-source repos & issue tracker"),
            3,
            github_icon.clone(),
            false,
        ),
        make_item(
            9,
            t("Rust 文档", "Rust Docs"),
            "https://doc.rust-lang.org",
            "https://doc.rust-lang.org",
            t("标准库与语言参考", "Standard library & language reference"),
            3,
            globe_icon,
            false,
        ),
        make_item(
            10,
            "Microsoft Store",
            "Microsoft.WindowsStore_8wekyb3d8bbwe!App",
            "Microsoft.WindowsStore_8wekyb3d8bbwe!App",
            t("系统应用商店", "Windows Store app"),
            5,
            store_icon,
            false,
        ),
        make_item(
            11,
            t("画图", "Paint"),
            r"D:\Tools\Shortcuts\Paint.lnk",
            r"D:\Tools\Shortcuts\Paint.lnk",
            t("快速标注与简单修图", "Quick markup & image editing"),
            4,
            paint_icon,
            false,
        ),
        make_item(
            12,
            t("计算器", "Calculator"),
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            t("科学计算与程序员模式", "Scientific & programmer calculator"),
            5,
            calc_icon,
            false,
        ),
    ];

    let search_results = vec![
        make_item(
            1,
            "VS Code",
            t("常用 / 日常应用", "Common / Daily Apps"),
            r"C:\Program Files\Microsoft VS Code\Code.exe",
            t("主力代码编辑器", "Primary code editor"),
            0,
            code_icon.clone(),
            false,
        ),
        make_item(
            2,
            t("Windows 终端", "Windows Terminal"),
            t("常用 / 开发工具", "Common / Dev Tools"),
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
            t("PowerShell 与 WSL 终端", "PowerShell & WSL terminal"),
            5,
            term_icon,
            false,
        ),
        make_item(
            6,
            t("项目工作区", "Projects"),
            t("常用 / 常用目录", "Common / Folders"),
            r"D:\Workspace\Projects",
            t("本地源码仓库目录", "Local source code repositories"),
            2,
            folder_icon,
            false,
        ),
        make_item(
            8,
            "GitHub",
            t("常用 / 在线文档", "Common / Web Docs"),
            "https://github.com",
            t("开源仓库与 Issue 跟踪", "Open-source repos & issue tracker"),
            3,
            github_icon.clone(),
            false,
        ),
    ];

    let batch_nodes = vec![
        DestinationNode {
            id: 1,
            parent: 0,
            name: t("常用", "Common").into(),
            expanded: true,
        },
        DestinationNode {
            id: 11,
            parent: 1,
            name: t("日常应用", "Daily Apps").into(),
            expanded: false,
        },
        DestinationNode {
            id: 12,
            parent: 1,
            name: t("开发工具", "Dev Tools").into(),
            expanded: false,
        },
        DestinationNode {
            id: 13,
            parent: 1,
            name: t("常用目录", "Folders").into(),
            expanded: false,
        },
        DestinationNode {
            id: 2,
            parent: 0,
            name: t("开发", "Dev").into(),
            expanded: true,
        },
        DestinationNode {
            id: 21,
            parent: 2,
            name: t("编辑器与 IDE", "Editors & IDEs").into(),
            expanded: false,
        },
        DestinationNode {
            id: 22,
            parent: 2,
            name: t("命令行与调试", "CLI & Debug").into(),
            expanded: false,
        },
        DestinationNode {
            id: 3,
            parent: 0,
            name: t("效率", "Productivity").into(),
            expanded: false,
        },
    ];

    let conversion_scopes = vec![
        ScopeNode {
            id: 1,
            label: t("常用", "Common").into(),
            depth: 0,
            state: 2,
        },
        ScopeNode {
            id: 11,
            label: t("日常应用", "Daily Apps").into(),
            depth: 1,
            state: 2,
        },
        ScopeNode {
            id: 12,
            label: t("开发工具", "Dev Tools").into(),
            depth: 1,
            state: 2,
        },
        ScopeNode {
            id: 13,
            label: t("常用目录", "Folders").into(),
            depth: 1,
            state: 2,
        },
        ScopeNode {
            id: 2,
            label: t("开发", "Dev").into(),
            depth: 0,
            state: 1,
        },
        ScopeNode {
            id: 21,
            label: t("编辑器与 IDE", "Editors & IDEs").into(),
            depth: 1,
            state: 2,
        },
        ScopeNode {
            id: 22,
            label: t("命令行与调试", "CLI & Debug").into(),
            depth: 1,
            state: 0,
        },
        ScopeNode {
            id: 3,
            label: t("效率", "Productivity").into(),
            depth: 0,
            state: 0,
        },
    ];

    let list_items = grid_items.clone();

    PreviewData {
        groups: vec![
            nav(1, t("常用", "Common")),
            nav(2, t("开发", "Dev")),
            nav(3, t("效率", "Productivity")),
            nav(4, t("工具", "Tools")),
        ],
        categories: vec![
            nav(11, t("日常应用", "Daily Apps")),
            nav(12, t("开发工具", "Dev Tools")),
            nav(13, t("常用目录", "Folders")),
            nav(14, t("在线文档", "Web Docs")),
        ],
        active_group: 1,
        active_category: 11,
        grid_items,
        list_items,
        search_query: "code".into(),
        search_results,
        batch_nodes,
        discover_items: default_discover_items(en),
        broken_items: default_broken_items(en),
        conversion_scopes,
        picker_apps: default_picker_apps(en),
        protected_apps: vec![
            r"D:\Games\osu!\osu!.exe".into(),
            r"D:\SteamLibrary\steamapps\common\Cyberpunk 2077\bin\x64\Cyberpunk2077.exe".into(),
        ],
        data_path: r"D:\Tools\Raptim".into(),
        license_lines: default_license_lines(),
        notice_items: default_notice_items(en),
        editor_program_icon: code_icon,
        editor_url_icon: github_icon,
    }
}

fn preview_from_export(export: Export, base_dir: &Path, en: bool) -> PreviewData {
    let mut data = demo_data(en);
    let collections: Vec<Collection> = export.collections;
    let entries: Vec<Entry> = export.entries;

    let groups: Vec<NavItem> = collections
        .iter()
        .filter(|c| c.parent_id.is_none())
        .map(|c| nav(c.id as i32, &c.name))
        .collect();
    if groups.is_empty() {
        return data;
    }

    let active_group = groups[0].id;
    let categories: Vec<NavItem> = collections
        .iter()
        .filter(|c| c.parent_id == Some(i64::from(active_group)))
        .map(|c| nav(c.id as i32, &c.name))
        .collect();
    let active_category = categories.first().map_or(0, |c| c.id);

    let full_name = |cid: i64| -> String {
        let Some(cat) = collections.iter().find(|c| c.id == cid) else {
            return String::new();
        };
        match cat
            .parent_id
            .and_then(|pid| collections.iter().find(|p| p.id == pid))
        {
            Some(p) => format!("{} / {}", p.name, cat.name),
            None => cat.name.clone(),
        }
    };

    let mut grid_items = Vec::new();
    for (idx, e) in entries
        .iter()
        .filter(|e| e.collection_id == i64::from(active_category))
        .enumerate()
    {
        let icon = raptim::icons::display(&e.icon);
        let tip = raptim::paths::display_target(&e.target, base_dir);
        grid_items.push(make_item(
            e.id as i32,
            &e.name,
            e.target.value(),
            &tip,
            &e.notes,
            e.target.kind_index(),
            icon,
            idx < 3,
        ));
    }

    let mut search_results = Vec::new();
    let mut search_query = String::new();
    if let Some(first) = entries.first() {
        search_query = first.name.chars().take(4).collect();
        let mut index = raptim::search::SearchIndex::default();
        index.sync(&entries);
        for id in index.query(&search_query, &entries).into_iter().take(6) {
            if let Some(e) = entries.iter().find(|x| x.id == id) {
                let icon = raptim::icons::display(&e.icon);
                let tip = raptim::paths::display_target(&e.target, base_dir);
                search_results.push(make_item(
                    e.id as i32,
                    &e.name,
                    &full_name(e.collection_id),
                    &tip,
                    &e.notes,
                    e.target.kind_index(),
                    icon,
                    false,
                ));
            }
        }
    }

    let mut batch_nodes = Vec::new();
    let mut conversion_scopes = Vec::new();
    for (g_idx, grp) in collections
        .iter()
        .filter(|c| c.parent_id.is_none())
        .enumerate()
    {
        let expanded = g_idx < 2;
        batch_nodes.push(DestinationNode {
            id: grp.id as i32,
            parent: 0,
            name: grp.name.clone().into(),
            expanded,
        });
        conversion_scopes.push(ScopeNode {
            id: grp.id as i32,
            label: grp.name.clone().into(),
            depth: 0,
            state: if g_idx == 0 { 2 } else { 0 },
        });
        for child in collections.iter().filter(|c| c.parent_id == Some(grp.id)) {
            if expanded {
                batch_nodes.push(DestinationNode {
                    id: child.id as i32,
                    parent: grp.id as i32,
                    name: child.name.clone().into(),
                    expanded: false,
                });
            }
            conversion_scopes.push(ScopeNode {
                id: child.id as i32,
                label: child.name.clone().into(),
                depth: 1,
                state: if g_idx == 0 { 2 } else { 0 },
            });
        }
    }

    if let Some(first_prog) = entries
        .iter()
        .find(|e| matches!(e.target, Target::Program { .. }) && !e.icon.is_empty())
    {
        data.editor_program_icon = raptim::icons::display(&first_prog.icon);
    }
    if let Some(first_url) = entries
        .iter()
        .find(|e| matches!(e.target, Target::Url { .. }) && !e.icon.is_empty())
    {
        data.editor_url_icon = raptim::icons::display(&first_url.icon);
    }

    data.groups = groups;
    data.categories = categories;
    data.active_group = active_group;
    data.active_category = active_category;
    data.list_items = grid_items.clone();
    data.grid_items = grid_items;
    if !search_results.is_empty() {
        data.search_query = search_query;
        data.search_results = search_results;
    }
    if !batch_nodes.is_empty() {
        data.batch_nodes = batch_nodes;
    }
    if !conversion_scopes.is_empty() {
        data.conversion_scopes = conversion_scopes;
    }
    data.protected_apps = export.settings.protected_apps;
    data.data_path = raptim::paths::normal_path(base_dir)
        .to_string_lossy()
        .into_owned();
    data
}

/// Loads real entries and settings from an existing Raptim data directory without creating a new DB.
pub fn load_from_data_dir(dir: &Path, en: bool) -> Result<PreviewData, String> {
    let db_path = dir.join("raptim.db");
    if !db_path.is_file() {
        return Err(if en {
            format!(
                "raptim.db not found in data directory: {}",
                db_path.display()
            )
        } else {
            format!("数据目录中未找到 raptim.db：{}", db_path.display())
        });
    }
    let mut store = raptim::storage::Store::open(&db_path).map_err(|e| e.to_string())?;
    let export = store.take_initial().map_err(|e| e.to_string())?;
    Ok(preview_from_export(export, dir, en))
}

/// Loads real entries and settings from an exported Raptim JSON file.
pub fn load_from_json(path: &Path, en: bool) -> Result<PreviewData, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        if en {
            format!("Failed to read JSON: {e}")
        } else {
            format!("读取 JSON 失败：{e}")
        }
    })?;
    let export: Export = serde_json::from_str(&text).map_err(|e| {
        if en {
            format!("Failed to parse JSON: {e}")
        } else {
            format!("解析 JSON 失败：{e}")
        }
    })?;
    raptim::storage::validate_export(&export).map_err(|e| e.to_string())?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(preview_from_export(export, base, en))
}
