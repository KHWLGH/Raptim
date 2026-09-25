//! Offscreen pixel audit of the real Tooltip popup path (styled ToolTipImpl + HintSurface).
//!
//! Unlike `tooltip_render.rs`, which renders `HintSurface` directly, this test drives the
//! runtime popup machinery: hover a TouchArea, let the 500 ms tooltip timer fire and audit
//! the composited popup pixels the way the running application presents them.
use slint::{
    ComponentHandle,
    platform::{Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc, time::Duration};

mod popup_case {
    slint::slint! {
        import { Palette } from "std-widgets.slint";
        export { Palette }
        import { HintSurface } from "../ui/chrome.slint";
        export component PopupCase inherits Window {
            background: Palette.background;
            out property <color> probe-canvas: Palette.background;
            out property <color> probe-stroke: Palette.background.mix(Palette.foreground, 0.87);
            out property <color> probe-surface: Palette.background.mix(#ffffff, 0.975);
            out property <bool> touch-hover: touch.has-hover;
            Rectangle {
                x: 20px; y: 20px; width: 140px; height: 44px; background: transparent;
                touch := TouchArea {
                    Tooltip {
                        HintSurface {
                            title: "代码编辑器";
                            detail: "C:\\Apps\\Editor\\editor.exe";
                            notes: "项目备注";
                        }
                    }
                }
            }
        }
    }
}

type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct TestPlatform(Windows, std::time::Instant);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(Default::default());
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        self.1.elapsed()
    }
}

const WIDTH: usize = 380;
const HEIGHT: usize = 240;

fn render(window: &Rc<MinimalSoftwareWindow>) -> Vec<slint::Rgb8Pixel> {
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

fn near(actual: (u8, u8, u8), expected: slint::Color, tolerance: i32) -> bool {
    let (r, g, b) = (
        expected.red() as i32,
        expected.green() as i32,
        expected.blue() as i32,
    );
    (actual.0 as i32 - r).abs() <= tolerance
        && (actual.1 as i32 - g).abs() <= tolerance
        && (actual.2 as i32 - b).abs() <= tolerance
}

fn tooltip_popup_open(window: &slint::Window) -> bool {
    use i_slint_core::window::{WindowInner, WindowKind};
    WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .any(|p| matches!(p.window_kind, WindowKind::ToolTip))
}

/// Bounding box of every pixel that deviates from the canvas color.
fn content_bbox(
    pixels: &[slint::Rgb8Pixel],
    canvas: slint::Color,
) -> Option<(usize, usize, usize, usize)> {
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if !near(at(pixels, x, y), canvas, 24) {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    bbox
}

/// set_platform 每进程只能成功一次；所有用例共享同一组窗口。
fn dump_png(pixels: &[slint::Rgb8Pixel], scheme: &str, scale: f32) {
    if std::env::var_os("DIASCIA_TOOLTIP_DUMP").is_none() {
        return;
    }
    let path =
        std::path::Path::new("target").join(format!("tooltip-popup-{scheme}-{scale:.2}.png"));
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

#[test]
fn tooltip_popup_path_paints_clean_corners_over_canvas() {
    let windows: Windows = Default::default();
    slint::platform::set_platform(Box::new(TestPlatform(
        windows.clone(),
        std::time::Instant::now(),
    )))
    .unwrap();
    for (scheme, label) in [
        (i_slint_core::items::ColorScheme::Light, "light"),
        (i_slint_core::items::ColorScheme::Dark, "dark"),
    ] {
        // 每种缩放使用全新组件：避免上一缩放遗留的弹层/状态串扰。
        for scale in [1.0f32, 1.5, 2.0] {
            run_popup_case(&windows, scheme, label, scale);
        }
    }
}

fn run_popup_case(
    windows: &Windows,
    scheme: i_slint_core::items::ColorScheme,
    label: &str,
    scale: f32,
) {
    let case = popup_case::PopupCase::new().unwrap();
    case.show().unwrap();
    let window = windows.borrow().last().unwrap().clone();
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));

    // Establish item geometry with a first frame before hit-testing hover.
    let _ = render(&window);

    // PointerMoved 的坐标是逻辑像素；换算到物理像素 (90, 42) 对应的悬停点。
    case.global::<popup_case::Palette>()
        .set_color_scheme(scheme);
    window.dispatch_event(slint::platform::WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(90. / scale, 42. / scale),
    });

    let mut opened = false;
    for i in 0..80 {
        if i % 8 == 0 {
            eprintln!(
                "{label}@{scale}: hover={} popups={}",
                case.get_touch_hover(),
                tooltip_popup_open(&window)
            );
        }
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(25));
        if tooltip_popup_open(&window) {
            opened = true;
            break;
        }
    }
    assert!(opened, "{label}@{scale}: 悬停后 tooltip 弹层应当打开");

    // The application clears the styled tooltip chrome before every frame.
    diascia::windowing::prepare_tooltips(&window);
    let pixels = render(&window);
    dump_png(&pixels, label, scale);

    let canvas = case.get_probe_canvas();
    let Some((x0, y0, x1, y1)) = content_bbox(&pixels, canvas) else {
        panic!("{label}@{scale}: tooltip 弹层没有渲染出任何内容");
    };
    assert!(
        x1 - x0 > 40 && y1 - y0 > 30,
        "{label}@{scale}: 弹层区域异常狭小: ({x0},{y0})-({x1},{y1})"
    );

    // 弹层四角不得出现纯黑像素（黑底板/黑双边）。
    for (x, y) in [
        (x0, y0),
        (x1, y0),
        (x0, y1),
        (x1, y1),
        (x0 + (x1 - x0) / 2, y0),
        (x0 + (x1 - x0) / 2, y1),
    ] {
        let p = at(&pixels, x, y);
        assert_ne!(
            p,
            (0, 0, 0),
            "{label}@{scale}: 弹层边缘出现纯黑像素 at ({x},{y})"
        );
        assert!(
            near(p, canvas, 96) || near(p, case.get_probe_surface(), 96),
            "{label}@{scale}: 弹层边缘像素 ({x},{y}) 既不是画布也不是表面/描边混合: {p:?} canvas={:?}",
            canvas
        );
    }

    // 圆角弧不得比“描边/画布中较暗者”再暗一大截——这是浅色主题四角发黑的特征
    // （弹层半透明圆角被二次预乘，弧线像素向黑色偏移）。半径越大越明显。
    let stroke = case.get_probe_stroke();
    let lum = |c: slint::Color| (c.red() as i32 + c.green() as i32 + c.blue() as i32) / 3;
    let ref_dark = lum(stroke).min(lum(canvas));
    let corner = (6. * scale) as usize + 4;
    for (cx, cy, dx, dy) in [
        (x0, y0, 1, 1),
        (x1, y0, -1, 1),
        (x0, y1, 1, -1),
        (x1, y1, -1, -1),
    ] {
        for i in 0..corner {
            for j in 0..corner {
                let x = (cx as isize + dx * i as isize) as usize;
                let y = (cy as isize + dy * j as isize) as usize;
                // 只看贴边 4px 的描边带，避开文字与内部填充。
                if i.max(j) > 4 {
                    continue;
                }
                let p = at(&pixels, x, y);
                let bright = (p.0 as i32 + p.1 as i32 + p.2 as i32) / 3;
                assert!(
                    bright > ref_dark - 24,
                    "{label}@{scale}: 圆角弧发黑 at ({x},{y}) 亮度 {bright} 远低于描边/画布 {ref_dark}"
                );
            }
        }
    }

    // 内容 bbox 外 3-5px 是 ToolTipImpl 的内边距带：清除样式底板后必须是纯画布色。
    // 灰底板（浅色 alternate-background 与画布仅差 8）会被这个紧公差抓住。
    for y in (y0.saturating_sub(5))..(y1 + 6).min(HEIGHT) {
        for x in (x0.saturating_sub(5))..(x1 + 6).min(WIDTH) {
            let outside = x + 3 < x0 || x > x1 + 3 || y + 3 < y0 || y > y1 + 3;
            if outside {
                let p = at(&pixels, x, y);
                assert!(
                    near(p, canvas, 5),
                    "{label}@{scale}: 内边距带出现非画布像素（样式底板未清除）at ({x},{y}): {p:?} canvas={:?}",
                    canvas
                );
            }
        }
    }

    // 弹层外沿一圈不允许纯黑。
    for y in y0.saturating_sub(3)..(y1 + 4).min(HEIGHT) {
        for x in x0.saturating_sub(3)..(x1 + 4).min(WIDTH) {
            let inside = x >= x0 && x <= x1 && y >= y0 && y <= y1;
            if !inside {
                let p = at(&pixels, x, y);
                assert_ne!(
                    p,
                    (0, 0, 0),
                    "{label}@{scale}: 弹层外沿出现纯黑像素 at ({x},{y})"
                );
            }
        }
    }
}
