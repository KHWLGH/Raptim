mod compose;
mod demo;
mod gallery;
mod scenes;

use compose::{FrameStyle, compose_overview, frame_screenshot};
use gallery::{GalleryMeta, RenderedEntry, write_gallery};
use i_slint_core::items::ColorScheme;
use i_slint_core::window::{WindowAdapter, WindowInner};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

struct PreviewPlatform {
    latest: Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}

impl slint::platform::Platform for PreviewPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let msw = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        *self.latest.borrow_mut() = Some(msw.clone());
        Ok(msw)
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

pub struct PlatformHandle {
    latest: Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}

impl PlatformHandle {
    fn install() -> Result<Self, String> {
        let latest = Rc::new(RefCell::new(None));
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(PreviewPlatform {
            latest: Rc::clone(&latest),
            clock: Rc::clone(&clock),
        }))
        .map_err(|e| format!("初始化无头渲染平台失败：{e}"))?;
        Ok(Self { latest, clock })
    }

    /// Renders a component strictly in memory with zero Win32 HWND creation or focus change.
    pub fn render_component<C: ComponentHandle>(
        &self,
        component: &C,
        dark: bool,
        scale: f32,
        width: f32,
        height: f32,
    ) -> Result<image::RgbImage, String> {
        let msw = self
            .latest
            .borrow_mut()
            .take()
            .ok_or_else(|| "未捕获到 MinimalSoftwareWindow 实例".to_string())?;

        let scheme = if dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        WindowInner::from_pub(msw.window())
            .context()
            .set_color_scheme(scheme);

        msw.window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        msw.set_size(slint::LogicalSize::new(width, height));
        component.show().map_err(|e| e.to_string())?;
        slint::platform::update_timers_and_animations();

        let phys = msw.size();
        if phys.width == 0 || phys.height == 0 {
            let _ = component.hide();
            return Err("渲染窗口物理尺寸为 0".into());
        }

        let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(phys.width, phys.height);
        // First pass settles layout-driven bindings (such as ListView column count).
        msw.request_redraw();
        msw.draw_if_needed(|renderer| {
            renderer.render(buffer.make_mut_slice(), phys.width as usize);
        });

        self.clock
            .set(self.clock.get() + Duration::from_millis(600));
        slint::platform::update_timers_and_animations();
        msw.request_redraw();
        msw.draw_if_needed(|renderer| {
            renderer.render(buffer.make_mut_slice(), phys.width as usize);
        });

        let _ = component.hide();

        image::RgbImage::from_raw(phys.width, phys.height, buffer.as_bytes().to_vec())
            .ok_or_else(|| "转换 RGB 图像缓冲失败".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThemeSelection {
    Light,
    Dark,
    Both,
}

impl ThemeSelection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Both => "both",
        }
    }
    fn includes_light(self) -> bool {
        matches!(self, Self::Light | Self::Both)
    }
    fn includes_dark(self) -> bool {
        matches!(self, Self::Dark | Self::Both)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preset {
    Showcase,
    All,
}

impl Preset {
    fn as_str(self) -> &'static str {
        match self {
            Self::Showcase => "showcase",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lang {
    Zh,
    En,
}

impl Lang {
    fn as_str(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    fn is_en(self) -> bool {
        matches!(self, Self::En)
    }

    fn locale(self) -> raptim::i18n::Locale {
        match self {
            Self::Zh => raptim::i18n::Locale::ZhCn,
            Self::En => raptim::i18n::Locale::EnUs,
        }
    }
}

#[derive(Debug, PartialEq)]
struct CliOptions {
    out_dir: PathBuf,
    lang: Lang,
    theme: ThemeSelection,
    preset: Preset,
    scenes: Option<Vec<String>>,
    scale: f32,
    frame: FrameStyle,
    data_dir: Option<PathBuf>,
    import_json: Option<PathBuf>,
    no_gallery: bool,
    no_overview: bool,
    list: bool,
    help: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            out_dir: PathBuf::from("target/ui-previews"),
            lang: Lang::Zh,
            theme: ThemeSelection::Both,
            preset: Preset::Showcase,
            scenes: None,
            scale: 2.0,
            frame: FrameStyle::Window,
            data_dir: None,
            import_json: None,
            no_gallery: false,
            no_overview: false,
            list: false,
            help: false,
        }
    }
}

fn parse_args<I>(iter: I) -> Result<CliOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut opts = CliOptions::default();
    let mut args = iter.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => opts.help = true,
            "--list" => opts.list = true,
            "--no-gallery" => opts.no_gallery = true,
            "--no-overview" => opts.no_overview = true,
            "--out" => {
                let val = args.next().ok_or("--out 缺少目录参数")?;
                opts.out_dir = PathBuf::from(val);
            }
            "--lang" => {
                let val = args.next().ok_or("--lang 缺少参数 (zh|en)")?;
                opts.lang = match val.to_ascii_lowercase().as_str() {
                    "zh" | "zh-cn" => Lang::Zh,
                    "en" | "en-us" => Lang::En,
                    other => return Err(format!("无效语言：{other}（可选 zh|en）")),
                };
            }
            "--theme" => {
                let val = args.next().ok_or("--theme 缺少参数 (light|dark|both)")?;
                opts.theme = match val.as_str() {
                    "light" => ThemeSelection::Light,
                    "dark" => ThemeSelection::Dark,
                    "both" => ThemeSelection::Both,
                    other => return Err(format!("无效主题：{other}（可选 light|dark|both）")),
                };
            }
            "--preset" => {
                let val = args.next().ok_or("--preset 缺少参数 (showcase|all)")?;
                opts.preset = match val.as_str() {
                    "showcase" => Preset::Showcase,
                    "all" => Preset::All,
                    other => return Err(format!("无效预设：{other}（可选 showcase|all）")),
                };
            }
            "--scenes" => {
                let val = args.next().ok_or("--scenes 缺少场景列表")?;
                let list: Vec<String> = val
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                if list.is_empty() {
                    return Err("--scenes 不能为空".into());
                }
                opts.scenes = Some(list);
            }
            "--scale" => {
                let val = args.next().ok_or("--scale 缺少数值参数")?;
                let scale: f32 = val.parse().map_err(|_| format!("无效缩放比例：{val}"))?;
                if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
                    return Err("缩放比例必须在 0.5 到 4.0 之间".into());
                }
                opts.scale = scale;
            }
            "--frame" => {
                let val = args.next().ok_or("--frame 缺少参数 (window|card|raw)")?;
                opts.frame = match val.as_str() {
                    "window" => FrameStyle::Window,
                    "card" => FrameStyle::Card,
                    "raw" => FrameStyle::Raw,
                    other => return Err(format!("无效边框样式：{other}（可选 window|card|raw）")),
                };
            }
            "--data-dir" => {
                let val = args.next().ok_or("--data-dir 缺少目录参数")?;
                opts.data_dir = Some(PathBuf::from(val));
            }
            "--import-json" => {
                let val = args.next().ok_or("--import-json 缺少文件路径")?;
                opts.import_json = Some(PathBuf::from(val));
            }
            other => return Err(format!("未知参数：{other}（使用 --help 查看帮助）")),
        }
    }
    if opts.data_dir.is_some() && opts.import_json.is_some() {
        return Err("--data-dir 与 --import-json 不能同时使用".into());
    }
    Ok(opts)
}

fn print_help() {
    println!(
        "Raptim 无头界面预览生成工具（不弹出/不抢占前台窗口）

用法:
  cargo run --release --locked --example ui_preview -- [选项]

选项:
  --out <目录>            输出目录（默认: target/ui-previews）
  --lang <语言>           zh | en（默认: zh）
  --theme <模式>          light | dark | both（默认: both）
  --preset <预设>         showcase | all（默认: showcase）
  --scenes <id1,id2,...>  只渲染指定场景（覆盖 --preset）
  --scale <倍率>          DPI 缩放倍率 0.5..=4.0（默认: 2.0）
  --frame <样式>          window | card | raw（默认: window）
  --data-dir <目录>       从现有 Raptim 数据目录读取真实项目（不新建空库）
  --import-json <文件>    从导出的 Raptim JSON 读取真实项目
  --no-gallery            不生成 index.html 与 manifest.json
  --no-overview           不生成总览拼图 overview-*.png
  --list                  列出全部可用预览场景并退出
  -h, --help              显示此帮助"
    );
}

fn print_scenes(lang: Lang) {
    let en = lang.is_en();
    if en {
        println!(
            "{:<24} {:<12} {:<10} {:<11} Title",
            "ID", "Group", "Showcase", "Logical"
        );
    } else {
        println!(
            "{:<24} {:<10} {:<10} {:<11} 标题",
            "ID", "分组", "默认精选", "逻辑尺寸"
        );
    }
    for s in scenes::SCENES {
        let showcase = if en {
            if s.in_showcase { "yes" } else { "no" }
        } else if s.in_showcase {
            "是"
        } else {
            "否"
        };
        println!(
            "{:<24} {:<12} {:<10} {:>3}x{:<6} {}",
            s.id,
            s.group_for(en),
            showcase,
            s.width as u32,
            s.height as u32,
            s.title_for(en)
        );
    }
}

fn resolve_scenes(opts: &CliOptions) -> Result<Vec<&'static scenes::SceneSpec>, String> {
    if let Some(ids) = &opts.scenes {
        let mut selected = Vec::with_capacity(ids.len());
        for id in ids {
            let spec = scenes::SCENES
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| format!("未知场景 ID：{id}（可用 --list 查看全部场景）"))?;
            selected.push(spec);
        }
        return Ok(selected);
    }
    Ok(scenes::SCENES
        .iter()
        .filter(|s| opts.preset == Preset::All || s.in_showcase)
        .collect())
}

fn save_png(path: &Path, img: &image::RgbaImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}：{e}", parent.display()))?;
    }
    img.save(path)
        .map_err(|e| format!("保存图片失败 {}：{e}", path.display()))
}

fn run() -> Result<(), String> {
    let opts = parse_args(std::env::args().skip(1))?;
    if opts.help {
        print_help();
        return Ok(());
    }
    if opts.list {
        print_scenes(opts.lang);
        return Ok(());
    }

    raptim::i18n::set_locale(opts.lang.locale());
    let en = opts.lang.is_en();
    let selected = resolve_scenes(&opts)?;
    let platform = PlatformHandle::install()?;

    let data = if let Some(dir) = &opts.data_dir {
        demo::load_from_data_dir(dir, en)?
    } else if let Some(json) = &opts.import_json {
        demo::load_from_json(json, en)?
    } else {
        demo::demo_data(en)
    };

    std::fs::create_dir_all(&opts.out_dir)
        .map_err(|e| format!("创建输出目录失败 {}：{e}", opts.out_dir.display()))?;

    let mut light_tiles: Vec<(&'static str, image::RgbaImage)> = Vec::new();
    let mut dark_tiles: Vec<(&'static str, image::RgbaImage)> = Vec::new();
    let mut entries = Vec::with_capacity(selected.len());

    for scene in &selected {
        let mut light_rel = None;
        let mut dark_rel = None;
        let mut framed_w = 0;
        let mut framed_h = 0;

        if opts.theme.includes_light() {
            let raw = scenes::render_scene(scene, &data, false, opts.scale, en, &platform)?;
            let framed = frame_screenshot(&raw, false, opts.scale, opts.frame);
            framed_w = framed.width();
            framed_h = framed.height();
            let rel = format!("light/{}.png", scene.id);
            save_png(&opts.out_dir.join(&rel), &framed)?;
            light_rel = Some(rel);
            light_tiles.push((scene.id, framed));
        }

        if opts.theme.includes_dark() {
            let raw = scenes::render_scene(scene, &data, true, opts.scale, en, &platform)?;
            let framed = frame_screenshot(&raw, true, opts.scale, opts.frame);
            framed_w = framed.width();
            framed_h = framed.height();
            let rel = format!("dark/{}.png", scene.id);
            save_png(&opts.out_dir.join(&rel), &framed)?;
            dark_rel = Some(rel);
            dark_tiles.push((scene.id, framed));
        }

        entries.push(RenderedEntry {
            id: scene.id.into(),
            group: scene.group_for(en).into(),
            title: scene.title_for(en).into(),
            description: scene.description_for(en).into(),
            light_rel,
            dark_rel,
            width: framed_w,
            height: framed_h,
        });
    }

    let mut overview_light = None;
    let mut overview_dark = None;
    if !opts.no_overview {
        if !light_tiles.is_empty() {
            let refs: Vec<(&str, &image::RgbaImage)> =
                light_tiles.iter().map(|(id, img)| (*id, img)).collect();
            if let Some(overview) = compose_overview(&refs, false, opts.scale) {
                save_png(&opts.out_dir.join("overview-light.png"), &overview)?;
                overview_light = Some("overview-light.png".into());
            }
        }
        if !dark_tiles.is_empty() {
            let refs: Vec<(&str, &image::RgbaImage)> =
                dark_tiles.iter().map(|(id, img)| (*id, img)).collect();
            if let Some(overview) = compose_overview(&refs, true, opts.scale) {
                save_png(&opts.out_dir.join("overview-dark.png"), &overview)?;
                overview_dark = Some("overview-dark.png".into());
            }
        }
    }

    let preset_label = if opts.scenes.is_some() {
        "custom"
    } else {
        opts.preset.as_str()
    };
    if !opts.no_gallery {
        let meta = GalleryMeta {
            app_version: env!("CARGO_PKG_VERSION").into(),
            lang: opts.lang.as_str().into(),
            preset: preset_label.into(),
            theme: opts.theme.as_str().into(),
            frame: opts.frame.as_str().into(),
            scale: opts.scale,
            overview_light: overview_light.clone(),
            overview_dark: overview_dark.clone(),
        };
        write_gallery(&opts.out_dir, &entries, &meta)?;
    }

    let total_pngs = light_tiles.len()
        + dark_tiles.len()
        + usize::from(overview_light.is_some())
        + usize::from(overview_dark.is_some());
    if en {
        println!(
            "Rendered {total_pngs} preview image(s) ({} scene(s)) -> {}",
            entries.len(),
            opts.out_dir.display()
        );
        if !opts.no_gallery {
            println!(
                "Gallery page: {}",
                opts.out_dir.join("index.html").display()
            );
        }
    } else {
        println!(
            "已无头生成 {total_pngs} 张预览图（{} 个场景）-> {}",
            entries.len(),
            opts.out_dir.display()
        );
        if !opts.no_gallery {
            println!("图库页面：{}", opts.out_dir.join("index.html").display());
        }
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("ui_preview: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn parses_defaults_and_flags() {
        let opts = parse_args(Vec::<String>::new()).unwrap();
        assert_eq!(opts, CliOptions::default());

        let parsed = parse_args(s(&[
            "--out",
            "out/dir",
            "--lang",
            "en",
            "--theme",
            "dark",
            "--preset",
            "all",
            "--scale",
            "1.5",
            "--frame",
            "card",
            "--no-gallery",
            "--no-overview",
        ]))
        .unwrap();
        assert_eq!(parsed.out_dir, PathBuf::from("out/dir"));
        assert_eq!(parsed.lang, Lang::En);
        assert_eq!(parsed.theme, ThemeSelection::Dark);
        assert_eq!(parsed.preset, Preset::All);
        assert_eq!(parsed.scale, 1.5);
        assert_eq!(parsed.frame, FrameStyle::Card);
        assert!(parsed.no_gallery);
        assert!(parsed.no_overview);
        assert_eq!(parse_args(s(&["--lang", "EN-US"])).unwrap().lang, Lang::En);
        assert_eq!(parse_args(s(&["--lang", "zh-CN"])).unwrap().lang, Lang::Zh);
    }

    #[test]
    fn parses_scenes_list_and_rejects_invalid_inputs() {
        let parsed = parse_args(s(&["--scenes", "main-grid, settings-about"])).unwrap();
        assert_eq!(
            parsed.scenes.as_deref(),
            Some(&["main-grid".to_string(), "settings-about".to_string()][..])
        );
        assert_eq!(resolve_scenes(&parsed).unwrap().len(), 2);

        assert!(parse_args(s(&["--lang", "fr"])).is_err());
        assert!(parse_args(s(&["--scale", "0.1"])).is_err());
        assert!(parse_args(s(&["--theme", "neon"])).is_err());
        assert!(parse_args(s(&["--frame", "none"])).is_err());
        assert!(parse_args(s(&["--data-dir", "a", "--import-json", "b"])).is_err());
    }

    #[test]
    fn supports_list_and_help() {
        assert!(parse_args(s(&["--list"])).unwrap().list);
        assert!(parse_args(s(&["-h"])).unwrap().help);
    }
}
