//! The software renderer must be able to repaint a single row without a full-frame dirty
//! region: normal frames (icon arrival, selection, search) rely on it, and only surface
//! replacement gets the explicit full repaint from `windowing`.
use slint::{
    ComponentHandle, Image, Model as ModelTrait, SharedPixelBuffer, VecModel,
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

const WIDTH: usize = 440;
const HEIGHT: usize = 640;

fn solid(red: u8, green: u8, blue: u8) -> Image {
    let bytes: Vec<u8> = (0..36 * 36).flat_map(|_| [red, green, blue, 255]).collect();
    Image::from_rgba8(SharedPixelBuffer::clone_from_slice(&bytes, 36, 36))
}
fn item(id: i32, name: &str, icon: Image) -> Item {
    Item {
        id,
        name: name.into(),
        detail: "x".into(),
        icon,
        ..Default::default()
    }
}
/// Pixels of one flat colour, which is what each tile's icon square is painted with here.
fn colour(pixels: &[slint::Rgb8Pixel], red: u8, green: u8, blue: u8) -> usize {
    pixels
        .iter()
        .filter(|p| p.r.abs_diff(red) < 24 && p.g.abs_diff(green) < 24 && p.b.abs_diff(blue) < 24)
        .count()
}

#[test]
fn row_update_repaints_itself_and_keeps_the_neighbour() {
    let windows: Windows = Default::default();
    slint::platform::set_platform(Box::new(TestPlatform(windows.clone()))).unwrap();
    let ui = MainWindow::new().unwrap();
    ui.show().unwrap();
    let window = windows.borrow()[0].clone();
    window.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    let model = Rc::new(VecModel::from(vec![
        item(1, "first", solid(0, 0, 255)),
        item(2, "second", solid(0, 255, 0)),
    ]));
    ui.set_items(model.clone().into());

    // One persistent surface, exactly like a real window's backing store.
    let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
    diascia::windowing::invalidate(ui.window());
    window.draw_if_needed(|r| {
        r.render(&mut pixels, WIDTH);
    });
    let blue_before = colour(&pixels, 0, 0, 255);
    let green_before = colour(&pixels, 0, 255, 0);
    assert!(
        blue_before > 200 && green_before > 200,
        "两个图标都要画出来：blue {blue_before} green {green_before}"
    );

    // A row-level change with nothing marking the whole frame dirty.
    model.set_row_data(0, item(1, "first", solid(255, 0, 0)));
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, WIDTH);
    });
    assert!(colour(&pixels, 255, 0, 0) > 200, "脏区必须重绘出新图标");
    assert_eq!(colour(&pixels, 0, 0, 255), 0, "旧图标不能残留");
    assert!(
        colour(&pixels, 0, 255, 0) >= green_before - 5,
        "脏区之外的内容必须保留"
    );
}
