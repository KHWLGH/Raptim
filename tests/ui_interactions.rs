use slint::{
    ComponentHandle, Model, VecModel,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};
slint::include_modules!();
type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct TestPlatform {
    windows: Windows,
    clock: Rc<Cell<Duration>>,
}
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(Default::default());
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}
#[test]
fn four_columns_menu_routes_drag_halves_toasts_and_settings() {
    let windows: Windows = Default::default();
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform {
        windows: windows.clone(),
        clock: clock.clone(),
    }))
    .unwrap();
    let ui = MainWindow::new().unwrap();
    ui.set_groups(
        Rc::new(VecModel::from(vec![
            NavItem {
                id: 1,
                name: "常用".into(),
            },
            NavItem {
                id: 3,
                name: "工作".into(),
            },
        ]))
        .into(),
    );
    ui.set_categories(
        Rc::new(VecModel::from(vec![NavItem {
            id: 2,
            name: "默认分类".into(),
        }]))
        .into(),
    );
    ui.set_category_id(2);
    ui.set_group_id(1);
    ui.set_items(
        Rc::new(VecModel::from(
            (1..=8)
                .map(|id| Item {
                    checked: false,
                    tooltip_path: "C:\\Apps\\Example.exe".into(),
                    tooltip_notes: "项目备注".into(),
                    id,
                    name: format!("长中文软件名称{id}").into(),
                    detail: "位置".into(),
                    kind: 0,
                    icon: Default::default(),
                    drag_icon: Default::default(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    let launches = Rc::new(Cell::new(0));
    let c = launches.clone();
    ui.on_launch(move |_| c.set(c.get() + 1));
    let menu = Rc::new(Cell::new((0, 0)));
    let c = menu.clone();
    ui.on_native_menu(move |kind, id| c.set((kind, id)));
    ui.on_make_drag(|id| {
        let mut data = slint::DataTransfer::default();
        data.set_user_data(Rc::new(id));
        data
    });
    ui.on_can_drop(|data| data.user_data().is_some());
    let drop_target = Rc::new(Cell::new(None));
    let c = drop_target.clone();
    ui.on_drop_before(move |data, id, after| {
        c.set(Some((
            *data.user_data().unwrap().downcast_ref::<i32>().unwrap(),
            id,
            after,
        )));
        true
    });
    ui.show().unwrap();
    let window = windows.borrow()[0].clone();
    window.set_size(slint::PhysicalSize::new(440, 640));
    let render = || {
        diascia::windowing::invalidate(ui.window());
        window.draw_if_needed(|r| {
            r.render(&mut vec![slint::Rgb8Pixel::default(); 440 * 640], 440);
        });
    };
    let click = |x, y, button| {
        for event in [
            WindowEvent::PointerPressed {
                position: slint::LogicalPosition::new(x, y),
                button,
            },
            WindowEvent::PointerReleased {
                position: slint::LogicalPosition::new(x, y),
                button,
            },
        ] {
            ui.window().dispatch_event(event);
        }
    };
    render();
    click(388., 145., PointerEventButton::Right);
    assert_eq!(
        menu.get(),
        (1, 4),
        "fourth item must be on first row; context uses stable ID"
    );
    assert_eq!(launches.get(), 0);
    click(42., 115., PointerEventButton::Right);
    assert_eq!(menu.get(), (3, 2));
    ui.invoke_show_more();
    assert_eq!(menu.get(), (0, 0));
    let drag = |from: (f32, f32), to: (f32, f32)| {
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(from.0, from.1),
            button: PointerEventButton::Left,
        });
        for i in 1..=8 {
            let t = i as f32 / 8.;
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(
                    from.0 + (to.0 - from.0) * t,
                    from.1 + (to.1 - from.1) * t,
                ),
            });
        }
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(to.0, to.1),
            button: PointerEventButton::Left,
        });
    };
    drag((128., 145.), (250., 145.));
    assert_eq!(
        drop_target.get(),
        Some((1, 2, true)),
        "forward drop targets the right half"
    );
    assert_eq!(launches.get(), 0);
    render();
    drag((387., 145.), (112., 145.));
    assert_eq!(
        drop_target.get(),
        Some((4, 1, false)),
        "backward drop targets the left half"
    );
    assert_eq!(launches.get(), 0);
    ui.set_list_view(true);
    render();
    click(180., 190., PointerEventButton::Right);
    assert_eq!(menu.get(), (1, 2), "list rows use the same item commands");
    drag((180., 125.), (180., 211.));
    assert_eq!(
        drop_target.get(),
        Some((1, 2, true)),
        "list reorders along the vertical axis"
    );
    assert_eq!(launches.get(), 0);
    ui.set_list_view(false);
    ui.set_selected_id(4);
    ui.invoke_focus_search();
    ui.set_selected_id(8);
    ui.invoke_dismiss_search();
    assert_eq!(ui.get_selected_id(), 4);
    ui.set_status("添加成功".into());
    render();
    click(421., 620., PointerEventButton::Left);
    assert!(ui.get_status().is_empty(), "toast can be closed");
    ui.set_status("自动消失".into());
    render();
    clock.set(Duration::from_secs(7));
    slint::platform::update_timers_and_animations();
    assert!(ui.get_status().is_empty());
    let settings = SettingsWindow::new().unwrap();
    settings.show().unwrap();
    let sw = windows.borrow()[1].clone();
    sw.set_size(slint::PhysicalSize::new(660, 480));
    settings.set_page(1);
    settings.set_edge_hide(true);
    settings.set_edge_strip_visible(true);
    settings.set_edge_strip_width(6);
    let changes = Rc::new(Cell::new(0));
    let c = changes.clone();
    settings.on_change(move |_| c.set(c.get() + 1));
    sw.draw_if_needed(|r| {
        r.render(&mut vec![slint::Rgb8Pixel::default(); 660 * 480], 660);
    });
    for event in [
        WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(621., 267.),
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(621., 267.),
            button: PointerEventButton::Left,
        },
    ] {
        settings.window().dispatch_event(event);
    }
    assert_eq!(
        settings.get_edge_strip_width(),
        7,
        "aligned plus increments width"
    );
    assert_eq!(changes.get(), 1);
    for event in [
        WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(580., 267.),
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(580., 267.),
            button: PointerEventButton::Left,
        },
    ] {
        settings.window().dispatch_event(event);
    }
    let replace_width = |text: &str| {
        settings.window().dispatch_event(WindowEvent::KeyPressed {
            text: slint::platform::Key::Control.into(),
        });
        settings
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: "a".into() });
        settings
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: "a".into() });
        settings.window().dispatch_event(WindowEvent::KeyReleased {
            text: slint::platform::Key::Control.into(),
        });
        settings
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        settings
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    };
    replace_width("12");
    assert_eq!(
        settings.get_edge_strip_width(),
        12,
        "two-digit keyboard input commits"
    );
    replace_width("13");
    assert_eq!(
        settings.get_edge_strip_width(),
        12,
        "invalid input retains the last valid width"
    );
    settings.set_edge_strip_visible(false);
    slint::platform::update_timers_and_animations();
    settings.set_edge_strip_visible(true);
    assert_eq!(settings.get_edge_strip_width(), 12);
    settings.set_page(4);
    sw.draw_if_needed(|r| {
        r.render(&mut vec![slint::Rgb8Pixel::default(); 660 * 480], 660);
    });
    assert_eq!(windows.borrow().len(), 2);
    assert_eq!(ui.get_items().row_count(), 8);
    // Bulk mode remains an explicit overlay, and preserves single-click selection in double-click mode.
    ui.set_search_open(false);
    ui.set_list_view(false);
    ui.set_double_click(true);
    ui.set_batch_mode(true);
    ui.set_batch_count(2);
    render();
    let before = launches.get();
    click(135., 145., PointerEventButton::Left);
    assert_eq!(
        launches.get(),
        before + 1,
        "batch selection dispatches on a single click even in double-click launch mode"
    );
    let prior_menu = menu.get();
    let prior_drop = drop_target.get();
    click(135., 145., PointerEventButton::Right);
    assert_eq!(
        menu.get(),
        prior_menu,
        "single-item commands disabled in batch mode"
    );
    drag((128., 145.), (250., 145.));
    assert_eq!(drop_target.get(), prior_drop, "batch mode cannot reorder");
    ui.set_batch_nodes(
        Rc::new(VecModel::from(vec![
            DestinationNode {
                id: 1,
                parent: 0,
                name: "常用".into(),
                expanded: true,
            },
            DestinationNode {
                id: 2,
                parent: 1,
                name: "默认分类".into(),
                expanded: false,
            },
        ]))
        .into(),
    );
    let key = Rc::new(Cell::new(99));
    let c = key.clone();
    ui.on_batch_tree_key(move |direction| c.set(direction));
    ui.set_batch_target_open(true);
    ui.invoke_focus_batch_tree();
    render();
    for (text, direction) in [
        (slint::platform::Key::DownArrow, 1),
        (slint::platform::Key::RightArrow, 2),
        (slint::platform::Key::LeftArrow, -2),
        (slint::platform::Key::UpArrow, -1),
        (slint::platform::Key::Return, 0),
    ] {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
        assert_eq!(key.get(), direction);
    }
    ui.set_batch_target_open(false);
    ui.set_batch_mode(false);
    // Large grids are virtual rows and still route clicks to stable item IDs after scrolling.
    let items: Vec<_> = (1..=10000)
        .map(|id| Item {
            id,
            name: format!("项目{id}").into(),
            ..Default::default()
        })
        .collect();
    ui.set_items(Rc::new(VecModel::from(items)).into());
    ui.set_grid_scroll(-5200.);
    render();
    assert!(ui.get_grid_first() > 0);
    assert!(ui.get_grid_visible_count() < 100);
    click(135., 145., PointerEventButton::Right);
    assert!(
        menu.get().1 > 100,
        "scrolled virtual rows use actual item IDs"
    );
    let scroll = ui.get_grid_scroll();
    ui.set_search_open(true);
    render();
    ui.set_search_open(false);
    render();
    assert_eq!(
        ui.get_grid_scroll(),
        scroll,
        "search overlay preserves grid position"
    );
    ui.set_grid_scroll(0.);
    render();
    ui.window().dispatch_event(WindowEvent::PointerExited);
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(135., 145.),
    });
    slint::platform::update_timers_and_animations();
    clock.set(clock.get() + Duration::from_secs(2));
    slint::platform::update_timers_and_animations();
    assert!(
        !diascia::windowing::has_popup(ui.window()),
        "tooltips never pause edge hiding"
    );
    let calm = ui.get_interacting();
    ui.set_relocate_open(true);
    ui.set_relocate_name("计算器".into());
    ui.set_relocate_target("C:\\gone\\calc.exe".into());
    ui.set_relocate_message("找不到目标，可能已被删除或移动，可重新定位".into());
    render();
    assert!(
        ui.get_interacting() && !calm,
        "the relocate panel pauses edge and focus hiding"
    );
    ui.set_relocate_open(false);
    render();
    assert_eq!(ui.get_interacting(), calm);
    // An offscreen window never passes through the winit filter, so this is the layer that
    // proves the view itself refuses to act on a composition-confirming Enter.
    ui.set_inline_kind(3);
    ui.set_inline_text("新建分组".into());
    let saves = Rc::new(Cell::new(0));
    let counted = saves.clone();
    ui.on_commit_name(move || counted.set(counted.get() + 1));
    ui.invoke_focus_inline_name();
    render();
    let press = || {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: slint::platform::Key::Return.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: slint::platform::Key::Return.into(),
        });
    };
    ui.set_composing(true);
    press();
    assert_eq!(saves.get(), 0, "组合态回车不保存");
    ui.set_composing(false);
    press();
    assert_eq!(saves.get(), 1, "组合结束后回车正常保存");
}
