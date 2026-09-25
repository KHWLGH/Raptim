//! Offscreen audit of the batch-organising panel: the two rows must share one column grid, and
//! the move/copy/delete dialogs must highlight the button that carries out the action.
use slint::{
    ComponentHandle, VecModel,
    platform::{Platform, PlatformError, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc, time::Duration};
slint::include_modules!();

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

const WIDTH: usize = 900;
const HEIGHT: usize = 420;
/// The panel is pinned to the bottom of the window and holds two 32px button rows.
const PANEL_TOP: usize = HEIGHT - 88;
const ROW_HEIGHT: usize = 32;
const ROW_GAP: usize = 4;
/// A button border runs unbroken only through the middle of its height: the 4px corner radius eats
/// the first and last few rows, and the labels sit in the same band.
const BAND_INSET: usize = 4;
const BAND_HEIGHT: usize = 24;
/// The panel is inset by this much on both sides, and its own border is a vertical line too.
const PANEL_X: usize = 12;

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

fn luminance(p: &slint::Rgb8Pixel) -> i32 {
    (p.r as i32 + p.g as i32 + p.b as i32) / 3
}

/// Columns that are darker than the surrounding surfaces for the whole height of one button row.
/// A button border spans the row, glyph strokes only their cap height, so the height test keeps
/// the labels out of the result.
fn button_borders(pixels: &[slint::Rgb8Pixel], row: usize) -> Vec<usize> {
    let top = PANEL_TOP + 6 + BAND_INSET + row * (ROW_HEIGHT + ROW_GAP);
    (PANEL_X + 3..WIDTH - PANEL_X - 3)
        .filter(|&x| {
            (top..top + BAND_HEIGHT).all(|y| {
                let at =
                    |column: isize| luminance(&pixels[y * WIDTH + (x as isize + column) as usize]);
                at(0) < at(-2).max(at(2)) - 3
            })
        })
        .collect()
}

fn is_accent(p: &slint::Rgb8Pixel) -> bool {
    p.b as i32 > p.r as i32 + 40 && p.b as i32 > 120
}

/// Longest horizontal run of accent-filled pixels on the right half, which is where a dialog keeps
/// its buttons. The panel itself is never accent-filled, so this only measures the dialog.
fn widest_accent_run(pixels: &[slint::Rgb8Pixel]) -> usize {
    let mut best = 0;
    for y in 0..HEIGHT {
        let mut run = 0;
        for x in WIDTH / 2..WIDTH {
            run = if is_accent(&pixels[y * WIDTH + x]) {
                run + 1
            } else {
                0
            };
            best = best.max(run);
        }
    }
    best
}

#[test]
fn batch_panel_rows_share_one_column_grid_and_dialogs_highlight_their_action() {
    let windows: Windows = Default::default();
    slint::platform::set_platform(Box::new(TestPlatform(windows.clone()))).unwrap();
    let ui = MainWindow::new().unwrap();
    ui.set_groups(
        Rc::new(VecModel::from(vec![NavItem {
            id: 1,
            name: "常用".into(),
        }]))
        .into(),
    );
    ui.set_categories(
        Rc::new(VecModel::from(vec![NavItem {
            id: 2,
            name: "默认分类".into(),
        }]))
        .into(),
    );
    ui.set_group_id(1);
    ui.set_category_id(2);
    ui.set_items(
        Rc::new(VecModel::from(
            (1..=6)
                .map(|id| Item {
                    checked: id <= 2,
                    id,
                    name: format!("项目{id}").into(),
                    detail: "位置".into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    ui.set_batch_mode(true);
    ui.set_batch_count(2);
    ui.show().unwrap();
    let window = windows.borrow()[0].clone();

    let pixels = render(&window);
    let top = button_borders(&pixels, 0);
    let bottom = button_borders(&pixels, 1);
    assert_eq!(bottom.len(), 6, "下面一行应有三枚按钮：{bottom:?}");
    assert_eq!(top.len(), 4, "上面一行只有全选与完成：{top:?}");
    assert_eq!(
        top,
        bottom[2..],
        "全选/完成的列边界必须落在下面一行的列上：{top:?} vs {bottom:?}"
    );
    let widths = [1, 3, 5].map(|i| bottom[i] - bottom[i - 1]);
    assert!(
        widths.windows(2).all(|w| w[0].abs_diff(w[1]) <= 2),
        "三列必须等宽：{widths:?}"
    );
    assert!(
        widest_accent_run(&pixels) < 40,
        "没有弹窗时面板不该出现高亮按钮"
    );

    ui.set_batch_action(0);
    ui.set_batch_destination(2);
    ui.set_batch_target_name("默认分类".into());
    ui.set_batch_target_open(true);
    let pixels = render(&window);
    assert!(
        widest_accent_run(&pixels) >= 40,
        "移动到卡片里的执行按钮必须高亮"
    );

    ui.set_batch_target_open(false);
    ui.set_batch_confirm(true);
    let pixels = render(&window);
    assert!(
        widest_accent_run(&pixels) >= 40,
        "删除确认卡片里的删除按钮必须高亮"
    );
}
