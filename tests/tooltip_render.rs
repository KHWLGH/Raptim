//! Offscreen pixel audit for the shared tooltip surface in light and dark schemes.
use slint::{
    ComponentHandle,
    platform::{Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc, time::Duration};

mod hint_case {
    slint::slint! {
        import { Palette } from "std-widgets.slint";
        export { Palette }
        import { HintSurface } from "../ui/chrome.slint";
        export component HintCase inherits Window {
            background: Palette.background;
            // Mirror Theme.canvas / Theme.stroke / Theme.surface so the pixel audit
            // can compare against the theme itself instead of hard-coded brightness.
            out property <color> probe-canvas: Palette.background;
            out property <color> probe-stroke: Palette.background.mix(Palette.foreground, 0.87);
            out property <color> probe-surface: Palette.background.mix(#ffffff, 0.975);
            out property <length> content-height: surface.preferred-height;
            surface := HintSurface {
                x: 20px; y: 20px; width: 320px; height: surface.preferred-height;
                title: "长中文软件名称与完整路径提示";
                detail: "C:\\Portable\\Applications\\开发工具与项目文档资料\\VeryLongDirectoryWithoutSpaces\\Editor\\editor.exe";
                notes: "第一行备注\n第二行备注\n第三行备注\n第四行备注\n第五行备注\n第六行备注\n第七行必须省略";
            }
        }
    }
}

type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct TestPlatform(Windows);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(Default::default());
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        Duration::ZERO
    }
}

const WIDTH: usize = 360;
const HEIGHT: usize = 260;
const MARGIN: usize = 20;

fn render(window: &Rc<MinimalSoftwareWindow>) -> Vec<slint::Rgb8Pixel> {
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH);
    });
    pixels
}

fn at(pixels: &[slint::Rgb8Pixel], x: usize, y: usize) -> (u8, u8, u8) {
    let p = pixels[y * WIDTH + x];
    (p.r, p.g, p.b)
}

fn near(actual: (u8, u8, u8), expected: slint::Color) -> bool {
    let (r, g, b) = (
        expected.red() as i32,
        expected.green() as i32,
        expected.blue() as i32,
    );
    (actual.0 as i32 - r).abs() <= 24
        && (actual.1 as i32 - g).abs() <= 24
        && (actual.2 as i32 - b).abs() <= 24
}

#[test]
fn tooltip_edge_has_single_theme_stroke_without_black_outline() {
    let windows: Windows = Default::default();
    slint::platform::set_platform(Box::new(TestPlatform(windows.clone()))).unwrap();
    let hint = hint_case::HintCase::new().unwrap();
    hint.show().unwrap();
    let window = windows.borrow()[0].clone();
    for (scheme, label) in [
        (i_slint_core::items::ColorScheme::Light, "light"),
        (i_slint_core::items::ColorScheme::Dark, "dark"),
    ] {
        hint.global::<hint_case::Palette>().set_color_scheme(scheme);
        let pixels = render(&window);
        let canvas = hint.get_probe_canvas();
        eprintln!("{label}: content-height = {}", hint.get_content_height());
        if std::env::var_os("DIASCIA_TOOLTIP_DUMP").is_some() {
            let path = std::path::Path::new("target").join(format!("tooltip-direct-{label}.png"));
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
            let mut encoder = png::Encoder::new(
                std::fs::File::create(&path).unwrap(),
                WIDTH as u32,
                HEIGHT as u32,
            );
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let _ = encoder
                .write_header()
                .and_then(|mut w| w.write_image_data(&bytes));
        }

        // 表面顶缘（y = MARGIN）与表面内部参照点。
        let top_stroke = at(&pixels, WIDTH / 2, MARGIN);
        let interior = at(&pixels, WIDTH / 2, MARGIN + 10);
        let surface_bottom = MARGIN + hint.get_content_height().ceil() as usize;

        // 窗口四角必须是纯画布色：圆角外不允许黑角或黑框。
        for (x, y) in [
            (0, 0),
            (WIDTH - 1, 0),
            (0, HEIGHT - 1),
            (WIDTH - 1, HEIGHT - 1),
        ] {
            assert!(
                near(at(&pixels, x, y), canvas),
                "{label}: 窗口角落 ({x},{y}) 必须保持画布色，不能出现黑角或黑框"
            );
        }

        // 顶部中央必须恰好一层主题描边，表面只绘制一层主题背景。
        assert!(
            near(top_stroke, hint.get_probe_stroke()),
            "{label}: 顶部中央必须恰好一层主题描边"
        );
        assert!(
            near(interior, hint.get_probe_surface()),
            "{label}: 表面必须只绘制一层主题背景"
        );

        // 表面矩形（含描边）之外的整个窗口必须是纯画布色：
        // 不允许黑角、灰底板或任何残余描边。
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let inside_surface =
                    (MARGIN..MARGIN + 320).contains(&x) && y >= MARGIN && y < surface_bottom;
                if !inside_surface && !near(at(&pixels, x, y), canvas) {
                    panic!("{label}: 表面外出现非画布像素 at ({x},{y})");
                }
                if at(&pixels, x, y) == (0, 0, 0) {
                    panic!("{label}: 出现纯黑像素（黑角或黑框）at ({x},{y})");
                }
            }
        }
    }
}
