use slint::{
    ComponentHandle, VecModel,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{cell::RefCell, rc::Rc, time::Duration};
slint::include_modules!();
type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct TestPlatform {
    windows: Windows,
}
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(Default::default());
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        Duration::ZERO
    }
}

#[test]
fn titlebar_search_icon_expands_focuses_and_keeps_keyboard() {
    let windows: Windows = Default::default();
    slint::platform::set_platform(Box::new(TestPlatform {
        windows: windows.clone(),
    }))
    .unwrap();
    let ui = MainWindow::new().unwrap();
    let queries = Rc::new(RefCell::new(Vec::<String>::new()));
    let q = queries.clone();
    ui.on_search(move |text| q.borrow_mut().push(text.to_string()));
    let launches = Rc::new(RefCell::new(Vec::<i32>::new()));
    let l = launches.clone();
    ui.on_launch(move |id| l.borrow_mut().push(id));
    let batch_all = Rc::new(std::cell::Cell::new(0));
    let b = batch_all.clone();
    ui.on_batch_all(move || b.set(b.get() + 1));
    ui.show().unwrap();
    let window = windows.borrow()[0].clone();
    window.set_size(slint::PhysicalSize::new(440, 640));
    let render = || {
        diascia::windowing::invalidate(ui.window());
        window.draw_if_needed(|r| {
            r.render(&mut vec![slint::Rgb8Pixel::default(); 440 * 640], 440);
        });
    };
    let click = |x: f32, y: f32| {
        for event in [
            WindowEvent::PointerPressed {
                position: slint::LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position: slint::LogicalPosition::new(x, y),
                button: PointerEventButton::Left,
            },
        ] {
            ui.window().dispatch_event(event);
        }
    };
    let type_text = |text: &str| {
        for ch in text.chars() {
            let key: slint::SharedString = ch.to_string().into();
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text: key });
        }
    };
    render();
    assert!(!ui.get_search_open(), "默认状态只显示搜索图标入口");
    click(26., 22.);
    render();
    assert!(ui.get_search_open(), "点击图标展开标题栏搜索");
    assert!(ui.get_search_focused(), "展开后自动聚焦输入框");
    queries.borrow_mut().clear();
    type_text("jsq");
    assert_eq!(ui.get_query(), "jsq");
    assert_eq!(
        queries.borrow().as_slice(),
        ["j", "js", "jsq"],
        "输入即时触发搜索回调"
    );
    ui.set_search_scope(2);
    assert_eq!(ui.get_search_scope(), 2, "范围下拉保留全部/分组/分类");
    ui.invoke_search_scope_changed();
    ui.set_query("jsq".into());
    ui.invoke_clear_search();
    assert_eq!(ui.get_query(), "", "清除按钮清空输入并保持展开");
    assert!(ui.get_search_open());
    assert!(ui.get_search_focused());
    let scroll = ui.get_grid_scroll();
    ui.invoke_dismiss_search();
    assert!(!ui.get_search_open(), "关闭按钮收起搜索并恢复图标入口");
    assert_eq!(ui.get_grid_scroll(), scroll, "关闭后恢复原浏览位置");
    ui.set_selected_id(3);
    ui.invoke_focus_search();
    render();
    assert!(
        ui.get_search_open() && ui.get_search_focused(),
        "Ctrl+F 路径进入展开状态"
    );
    ui.set_results(
        Rc::new(VecModel::from(vec![
            Item {
                id: 11,
                name: "计算器".into(),
                detail: "常用 / 工具".into(),
                ..Default::default()
            },
            Item {
                id: 12,
                name: "记事本".into(),
                detail: "常用 / 工具".into(),
                ..Default::default()
            },
        ]))
        .into(),
    );
    ui.set_selected_id(12);
    ui.invoke_submit_search();
    assert_eq!(
        launches.borrow().as_slice(),
        [12],
        "Enter 启动当前选中的搜索结果"
    );
    ui.set_selected_id(0);
    ui.invoke_submit_search();
    assert_eq!(
        launches.borrow().as_slice(),
        [12, 11],
        "未选择时 Enter 启动首个结果"
    );
    ui.set_batch_mode(true);
    ui.set_batch_count(1);
    type_text("a");
    for event in [
        WindowEvent::KeyPressed {
            text: slint::platform::Key::Control.into(),
        },
        WindowEvent::KeyPressed { text: "a".into() },
        WindowEvent::KeyReleased { text: "a".into() },
        WindowEvent::KeyReleased {
            text: slint::platform::Key::Control.into(),
        },
    ] {
        ui.window().dispatch_event(event);
    }
    assert_eq!(
        batch_all.get(),
        0,
        "搜索输入框的 Ctrl+A 属于文本框，不被批量整理拦截"
    );
    assert!(ui.get_search_focused());
}

#[test]
fn about_page_pins_zero_version_and_blank_intro() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let slint_source = std::fs::read_to_string(format!("{manifest}\\ui\\app.slint")).unwrap();
    let about = slint_source
        .split("if root.page == 4:")
        .nth(1)
        .unwrap()
        .split("AboutSlint")
        .next()
        .unwrap();
    assert!(
        about.contains("\"Version 0.0.0\""),
        "关于页版本固定为 Version 0.0.0"
    );
    assert!(!about.contains("开发预览"), "关于页不再出现开发预览");
    let mut labels = Vec::new();
    let mut rest = about;
    while let Some(start) = rest.find("text: \"") {
        rest = &rest[start + 7..];
        let end = rest.find('"').unwrap();
        labels.push(rest[..end].to_string());
    }
    let allowed = [
        "Diascia",
        "Version 0.0.0",
        "Version ",
        "https://www.rust-lang.org/",
    ];
    for label in labels {
        assert!(
            allowed.contains(&label.as_str()),
            "关于页介绍区域必须保持空白，出现额外文案：{label}"
        );
    }
    let shown = about
        .split("Version ")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let cargo = std::fs::read_to_string(format!("{manifest}\\Cargo.toml")).unwrap();
    assert!(
        cargo.contains(&format!("version = \"{shown}\"")),
        "关于页显示 {shown}，Cargo 包版本必须与之一致"
    );
}
