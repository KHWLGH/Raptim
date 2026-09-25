//! Tests drive the application's real callbacks in an isolated native event loop.
//!
//! This is the machine pass: it puts the desktop to work. Startup shows the launcher with
//! `platform::focus`, four editor windows are shown and focused, a tray icon appears, and
//! `RegisterHotKey` claims Ctrl+Alt+Shift+F10 then F9 — so a running instance keeps its own F10
//! registration and fails here. Everything else in `tests/` is offscreen and never activates; keep
//! new checks that way unless they need a real HWND.
//!
//! Exactly one `#[test]` may exist in this file: the instance mutex and the native event loop are
//! both process-singletons, so a second one deadlocks the first. Extend the chain instead — add
//! `fn check_xxx(&Rc<RefCell<App>>)` and call it at the end of `check_picker_after_owner_close`,
//! before `check_window_state` (which mutates do-not-disturb state).
use super::*;

#[test]
#[ignore = "真实窗口/热键/托盘，会抢前台：cargo test -- --ignored"]
fn organize_protection_topmost_and_editor_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(&directory.path().join("diascia.db")).unwrap();
    let settings = Settings {
        fullscreen_protection: false,
        hide_on_focus_loss: false,
        hotkey: "Ctrl+Alt+Shift+F10".into(),
        ..Default::default()
    };
    store.save_settings(&settings).unwrap();
    store.save_collection(None, Some(1), "目标分类").unwrap();
    let group = store.save_collection(None, None, "第二分组").unwrap();
    let second = store
        .save_collection(None, Some(group), "第二分类")
        .unwrap();
    for id in 1..=3 {
        store
            .save_entry(&Entry {
                id: 0,
                collection_id: 2,
                name: format!("测试{id}"),
                aliases: String::new(),
                notes: "备注".into(),
                icon: vec![],
                position: 0,
                target: Target::Url {
                    url: format!("https://example.com/{id}"),
                },
            })
            .unwrap();
    }
    store
        .save_entry(&Entry {
            id: 0,
            collection_id: 2,
            name: "缺失工具".into(),
            aliases: String::new(),
            notes: String::new(),
            icon: vec![1, 2, 3],
            position: 0,
            target: Target::Program {
                path: "C:\\Regression\\missing-tool.exe".into(),
                arguments: "--keep".into(),
                working_dir: "C:\\Regression".into(),
                elevated: false,
                show: ShowMode::Minimized,
            },
        })
        .unwrap();
    // A packaged app kept in another category: its icon has to arrive through the startup
    // backfill, which is the only path that fills icons without the user asking. Addresses are
    // deliberately not part of that pass, so the fixture entries above keep no icon at all.
    let aumid = platform::installed_apps()
        .first()
        .expect("packaged app on this machine")
        .0
        .clone();
    store
        .save_entry(&Entry {
            id: 0,
            collection_id: second,
            name: "打包应用".into(),
            aliases: String::new(),
            notes: String::new(),
            icon: Vec::new(),
            position: 0,
            target: Target::App { aumid },
        })
        .unwrap();
    drop(store);
    run_at(directory.path().to_path_buf()).unwrap();
}

pub(super) fn install(app: Rc<RefCell<App>>) {
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let ui = app.borrow().main.clone_strong();
        let destination = app
            .borrow()
            .collections
            .iter()
            .find(|c| c.name == "目标分类")
            .unwrap()
            .id as i32;
        ui.invoke_batch_start(0);
        ui.invoke_launch(1);
        ui.invoke_batch_start(0);
        assert_eq!(ui.get_batch_count(), 1, "reentry preserves selection");
        ui.invoke_batch_target(0);
        assert_eq!(ui.get_batch_destination(), 0);
        assert!(ui.get_batch_target_open());
        assert!(ui.get_batch_nodes().iter().any(|n| n.parent == 1));
        ui.invoke_batch_node(1);
        assert_eq!(
            ui.get_batch_destination(),
            0,
            "groups are never destinations"
        );
        assert!(!ui.get_batch_nodes().iter().any(|n| n.parent == 1));
        ui.invoke_batch_tree_key(2);
        ui.invoke_batch_tree_key(1);
        assert_eq!(ui.get_batch_destination(), 2);
        ui.invoke_batch_tree_key(0);
        assert!(!ui.get_batch_error().is_empty(), "same category fails");
        assert!(ui.get_batch_target_open());
        assert_eq!(ui.get_batch_count(), 1);
        assert_eq!(ui.get_batch_destination(), 2);
        ui.invoke_escape();
        assert!(!ui.get_batch_target_open());
        assert!(ui.get_batch_mode());
        assert_eq!(ui.get_batch_count(), 1);
        ui.invoke_batch_target(1);
        ui.invoke_batch_node(destination);
        ui.invoke_batch_execute(1, destination);
        assert!(ui.get_batch_mode());
        assert_eq!(ui.get_batch_count(), 0);
        assert_eq!(app.borrow().collection, 2);
        ui.invoke_launch(1);
        ui.invoke_batch_target(1);
        ui.invoke_batch_node(destination);
        ui.invoke_batch_execute(1, destination);
        assert!(
            ui.get_batch_target_open(),
            "duplicate target leaves modal open"
        );
        assert_eq!(ui.get_batch_destination(), destination);
        assert_eq!(ui.get_batch_count(), 1);
        ui.set_query("测试".into());
        ui.invoke_search("测试".into());
        assert_eq!(ui.get_batch_count(), 0);
        assert!(!ui.get_batch_target_open());
        assert!(ui.get_batch_mode());
        ui.set_query("".into());
        ui.invoke_search("".into());
        ui.invoke_batch_all();
        assert_eq!(ui.get_batch_count(), 4);
        ui.invoke_batch_all();
        assert_eq!(ui.get_batch_count(), 0);
        ui.invoke_launch(2);
        ui.invoke_batch_target(0);
        ui.invoke_search_scope_changed();
        assert!(!ui.get_batch_target_open());
        assert_eq!(ui.get_batch_count(), 0);
        ui.invoke_launch(2);
        ui.invoke_batch_execute(0, destination);
        assert!(ui.get_batch_mode());
        assert_eq!(
            app.borrow()
                .entries
                .iter()
                .find(|e| e.id == 2)
                .unwrap()
                .collection_id,
            i64::from(destination)
        );
        ui.invoke_launch(3);
        ui.set_batch_confirm(true);
        ui.invoke_escape();
        assert_eq!(ui.get_batch_count(), 1);
        assert!(ui.get_batch_mode());
        ui.invoke_batch_execute(2, 0);
        assert!(!app.borrow().entries.iter().any(|e| e.id == 3));
        assert!(ui.get_batch_mode());
        assert_eq!(ui.get_batch_count(), 0);
        ui.invoke_batch_cancel();
        ui.set_selected_id(2);
        ui.invoke_focus_search();
        assert!(ui.get_search_open());
        assert!(ui.get_search_focused(), "展开搜索后输入框获得焦点");
        ui.set_selected_id(3);
        ui.invoke_escape();
        assert!(!ui.get_search_open(), "Esc 分层关闭搜索并恢复标题栏入口");
        assert_eq!(ui.get_query(), "");
        assert_eq!(ui.get_selected_id(), 2, "关闭搜索后键盘焦点回到原浏览位置");
        let failure = |entry: i64, relocatable: bool| {
            serde_json::to_string(&Failure {
                entry,
                message: "找不到目标，可能已被删除或移动，可重新定位".into(),
                relocatable,
            })
            .unwrap()
        };
        ui.invoke_launch_finished(failure(1, true).into());
        assert!(
            !ui.get_relocate_open(),
            "网址目标没有可重指的文件路径，只能留下提示"
        );
        let tool = app
            .borrow()
            .entries
            .iter()
            .find(|e| e.name == "缺失工具")
            .unwrap()
            .id;
        ui.invoke_launch_finished(failure(tool, true).into());
        assert!(ui.get_relocate_open(), "启动失败就地提供重定位");
        assert_eq!(ui.get_relocate_target(), "C:\\Regression\\missing-tool.exe");
        ui.invoke_escape();
        assert!(!ui.get_relocate_open(), "Esc 先关面板，不外泄成隐藏启动器");
        ui.invoke_item_action(tool as i32, 7);
        assert!(ui.get_relocate_open(), "右键菜单在提示消失后仍可进入");
        ui.invoke_file_selected(7, "C:\\Regression\\renamed-tool.exe".into());
        let moved = app
            .borrow()
            .entries
            .iter()
            .find(|e| e.id == tool)
            .unwrap()
            .clone();
        assert_eq!(
            moved.target,
            Target::Program {
                path: "C:\\Regression\\renamed-tool.exe".into(),
                arguments: "--keep".into(),
                working_dir: "C:\\Regression".into(),
                elevated: false,
                show: ShowMode::Minimized
            },
            "重定位只换路径"
        );
        assert!(moved.icon.is_empty(), "旧目标的图标不跟随新路径");
        assert!(!ui.get_relocate_open());
        open_editor(&app, 1);
        let generation = app.borrow().editor.as_ref().unwrap().generation;
        slint::Timer::single_shot(Duration::from_millis(150), move || {
            editor_ready(app, generation)
        });
    });
}

fn editor_ready(app: Rc<RefCell<App>>, old_generation: u64) {
    let old_ui = app.borrow().editor.as_ref().unwrap().window.clone_strong();
    {
        let mut s = app.borrow_mut();
        let editor = s.editor.as_ref().unwrap();
        assert!(
            editor._drop.is_some(),
            "first native HWND registered after show"
        );
        assert!(!editor.drop_failed);
        s.install_editor_drop(old_generation);
        assert!(
            !s.editor.as_ref().unwrap().drop_failed,
            "no duplicate registration"
        );
        s.drop_editor();
    }
    let released = platform::drop_target::Registration::new(old_ui.window(), |_| {})
        .expect("closing revokes old HWND registration");
    drop(released);
    drop(old_ui);
    open_editor(&app, 1);
    app.borrow_mut().drop_editor();
    open_editor(&app, 1);
    let ui = app.borrow().main.clone_strong();
    ui.invoke_native_event(12, old_generation.to_string().into());
    ui.invoke_native_event(
        13,
        serde_json::to_string(&(old_generation, "missing.png"))
            .unwrap()
            .into(),
    );
    assert!(
        app.borrow()
            .editor
            .as_ref()
            .unwrap()
            .window
            .get_error()
            .is_empty(),
        "stale drop cannot modify new editor"
    );
    slint::Timer::single_shot(Duration::from_millis(150), move || {
        assert!(
            app.borrow().editor.as_ref().unwrap()._drop.is_some(),
            "rapid reopen registered"
        );
        let path = app.borrow().data_dir.join("drop-image.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([50, 120, 200, 255]))
            .save(&path)
            .unwrap();
        let generation = app.borrow().editor.as_ref().unwrap().generation;
        let ui = app.borrow().main.clone_strong();
        ui.invoke_native_event(
            13,
            serde_json::to_string(&(generation, path.to_string_lossy()))
                .unwrap()
                .into(),
        );
        assert!(
            !app.borrow().editor.as_ref().unwrap().icon.is_empty(),
            "current drop updates icon draft"
        );
        assert!(
            app.borrow()
                .entries
                .iter()
                .find(|e| e.id == 1)
                .unwrap()
                .icon
                .is_empty(),
            "drop does not save before confirmation"
        );
        app.borrow_mut().drop_editor();
        check_app_picker(&app);
    });
}
/// The packaged-app picker is exercised against the real AppsFolder namespace, so this also
/// proves the enumeration reaches the view rather than only compiling.
fn check_app_picker(app: &Rc<RefCell<App>>) {
    open_editor(app, 1);
    let editor = app
        .borrow()
        .editor
        .as_ref()
        .expect("editor session")
        .window
        .clone_strong();
    editor.set_kind(5);
    editor.invoke_pick_app();
    assert!(editor.get_picker_open(), "选择按钮打开应用列表覆盖层");
    let shown = editor.get_picker_shown();
    let first = shown
        .row_data(0)
        .expect("AppsFolder enumeration reaches the picker");
    assert!(first.aumid.contains('!'), "{}", first.aumid);
    editor.set_picker_filter("zzz 不存在的应用名 zzz".into());
    editor.invoke_picker_refilter();
    assert_eq!(
        editor.get_picker_shown().row_count(),
        0,
        "the filter narrows the list rather than being ignored"
    );
    editor.set_picker_filter("".into());
    editor.invoke_picker_refilter();
    assert!(
        editor.get_picker_shown().row_count() > 0,
        "clearing restores it"
    );
    editor.invoke_picker_confirm(first.aumid.clone(), first.name.clone());
    assert_eq!(editor.get_target(), first.aumid, "选择结果回填编辑器");
    assert!(!editor.get_picker_open(), "确认后关闭选择器");
    assert!(app.borrow().icon_pending, "选定应用后立刻排队提取它的图标");
    let app = app.clone();
    let aumid = first.aumid.clone();
    let rows = shown.row_count();
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        check_packaged_icon(app, aumid, rows, 0)
    });
}

/// Icon extraction is an out-of-process Shell call, so its arrival is polled for rather than assumed
/// to fit one fixed window: a 400 ms hop flaked on a cold shell cache while the code under test was
/// fine, and a machine pass that fails at random is a machine pass people skip.
fn check_packaged_icon(
    app: Rc<RefCell<App>>,
    aumid: slint::SharedString,
    rows: usize,
    attempt: u32,
) {
    let ready = {
        let s = app.borrow();
        s.editor
            .as_ref()
            .is_some_and(|session| !session.icon.is_empty())
            && !s.icon_pending
    };
    if !ready {
        assert!(
            attempt < 20,
            "打包应用图标 {aumid} 在 4 秒内没有到达编辑器草稿"
        );
        let later = app.clone();
        slint::Timer::single_shot(Duration::from_millis(200), move || {
            check_packaged_icon(later, aumid, rows, attempt + 1)
        });
        return;
    }
    let editor = app
        .borrow()
        .editor
        .as_ref()
        .expect("图标结果到达前编辑器仍开着")
        .window
        .clone_strong();
    {
        let s = app.borrow();
        assert!(!s.icon_pending, "提取任务已收尾");
        assert!(
            editor.get_entry_icon().size().width > 0,
            "编辑器预览显示提取到的应用图标"
        );
        let seeded = s
            .entries
            .iter()
            .find(|e| e.name == "打包应用")
            .expect("fixture entry");
        assert!(
            !seeded.icon.is_empty(),
            "启动后台任务为打包应用项目补齐图标"
        );
    }
    editor.invoke_pick_app();
    assert!(editor.get_picker_open(), "可以再次打开列表");
    assert_eq!(
        editor.get_picker_shown().row_count(),
        rows,
        "重新打开时列表内容不变，筛选条件已复位"
    );
    app.borrow_mut().drop_editor();
    // The editor that hosted the list is gone now; give winit an event-loop turn before a
    // fresh editor tries to show it again.
    let later = app.clone();
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        check_picker_after_owner_close(&later);
    });
}

/// The list is an overlay inside the editor now, so destroying an editor cannot take a native
/// window of its own along. Reopening from a fresh editor still has to repopulate it.
fn check_picker_after_owner_close(app: &Rc<RefCell<App>>) {
    open_editor(app, 1);
    let editor = app
        .borrow()
        .editor
        .as_ref()
        .expect("editor session")
        .window
        .clone_strong();
    editor.set_kind(5);
    let later = app.clone();
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        editor.invoke_pick_app();
        assert!(editor.get_picker_open(), "编辑器销毁后重新打开仍然可用");
        assert!(
            editor.get_picker_shown().row_count() > 0,
            "重新打开仍然拿到应用列表"
        );
        assert_eq!(editor.get_picker_filter(), "", "重新打开时筛选条件复位");
        later.borrow_mut().drop_editor();
        check_collection_tree(&later);
        check_collection_delete(&later);
        check_window_state(&later);
        later.borrow_mut().request_quit();
    });
}

/// The editor's category chooser is the same tree the organizer uses, so it is exercised through
/// the same real callbacks rather than a parallel test path.
fn check_collection_tree(app: &Rc<RefCell<App>>) {
    open_editor(app, 1);
    let editor = app
        .borrow()
        .editor
        .as_ref()
        .expect("editor session")
        .window
        .clone_strong();
    let original = editor.get_collection_id();
    editor.invoke_pick_collection();
    assert!(editor.get_collection_open());
    let groups: Vec<_> = editor
        .get_collection_nodes()
        .iter()
        .filter(|node| node.parent == 0)
        .collect();
    assert!(!groups.is_empty(), "the tree lists groups");
    let collapsed = groups[0].expanded;
    editor.invoke_collection_node(groups[0].id);
    let after = editor
        .get_collection_nodes()
        .iter()
        .find(|node| node.id == groups[0].id)
        .expect("group row survives");
    assert_eq!(after.expanded, !collapsed, "a group row toggles expansion");
    assert_eq!(
        editor.get_collection_id(),
        original,
        "toggling a group never changes the choice"
    );
    editor.invoke_collection_node(groups[0].id);
    let category = editor
        .get_collection_nodes()
        .iter()
        .find(|node| node.parent != 0 && node.id != original)
        .expect("a selectable category");
    editor.invoke_collection_node(category.id);
    assert_eq!(editor.get_collection_id(), category.id);
    assert_eq!(editor.get_collection_name(), category.name);
    assert!(
        !editor.get_collection_open(),
        "picking closes the tree and returns to the form"
    );
    let draft = app.borrow().editor_draft().expect("draft");
    assert_eq!(draft.collection_id, i64::from(category.id));
    editor.invoke_pick_collection();
    editor.invoke_cancel();
    assert!(
        !editor.get_collection_open(),
        "Esc closes the tree instead of discarding the edit"
    );
    assert!(
        app.borrow().editor.is_some(),
        "the editor session survives that Esc"
    );
    assert!(
        !editor.get_discard_confirm(),
        "and it does not raise the discard prompt"
    );
    app.borrow_mut().drop_editor();
}

/// Deleting a group used to be impossible: the placeholder 默认分类 every group is given always
/// collided with the destination's own one, so `validate_export` rejected the whole migration
/// before a single row was written. Both removal modes run through the real popup callbacks here.
fn check_collection_delete(app: &Rc<RefCell<App>>) {
    let ui = app.borrow().main.clone_strong();
    let (first, second) = {
        let s = app.borrow();
        let group = |name: &str| {
            s.collections
                .iter()
                .find(|c| c.name == name && c.parent_id.is_none())
                .map(|c| c.id)
                .expect("fixture group")
        };
        (group("常用"), group("第二分组"))
    };
    ui.invoke_choose_group(first as i32);
    assert_eq!(app.borrow().group, first, "被删的正是当前选中的分组");
    ui.invoke_collection_action(first as i32, 3, 0);
    assert!(ui.get_collection_delete());
    assert!(ui.get_delete_is_group() && ui.get_delete_has_content());
    assert!(!ui.get_delete_discard());
    assert_eq!(ui.get_delete_destinations().row_count(), 1);
    ui.invoke_finish_delete(0);
    assert_eq!(
        ui.get_inline_error().as_str(),
        "",
        "同名默认分类不再让整次删除失败"
    );
    assert!(!ui.get_collection_delete());
    {
        let s = app.borrow();
        assert!(s.collections.iter().all(|c| c.id != first));
        assert!(
            s.collections
                .iter()
                .any(|c| c.name == "默认分类 2" && c.parent_id == Some(second)),
            "装着项目的默认分类改名保留"
        );
        assert_eq!(
            s.collections
                .iter()
                .filter(|c| c.name == "默认分类" && c.parent_id == Some(second))
                .count(),
            1,
            "目标分组仍然只有一个默认分类"
        );
        assert_eq!(s.group, second, "删掉当前分组后落到还在的分组上");
        assert_eq!(ui.get_group_id() as i64, s.group);
        assert_eq!(ui.get_category_id() as i64, s.collection);
        assert!(
            s.store.collections().unwrap().iter().all(|c| c.id != first),
            "数据库里也删掉了"
        );
    }
    let (temp, back_to) = {
        let mut s = app.borrow_mut();
        let temp = s
            .store
            .save_collection(None, Some(second), "临时分类")
            .unwrap();
        let mut draft = s.entries[0].clone();
        draft.id = 0;
        draft.name = "临时项目".into();
        draft.collection_id = temp;
        draft.target = Target::Url {
            url: "https://example.com/temp".into(),
        };
        s.store.save_entry(&draft).unwrap();
        let back_to = s.entries.len();
        s.report_refresh();
        (temp, back_to)
    };
    ui.invoke_collection_action(temp as i32, 3, 0);
    assert!(ui.get_collection_delete() && !ui.get_delete_is_group());
    assert!(ui.get_delete_has_content(), "有内容才谈得上不迁移");
    ui.set_delete_discard(true);
    ui.invoke_finish_delete(0);
    assert_eq!(ui.get_inline_error().as_str(), "");
    {
        let s = app.borrow();
        assert!(s.collections.iter().all(|c| c.id != temp));
        assert_eq!(s.entries.len(), back_to, "只有它带着的那条项目消失");
        assert!(s.collections.iter().any(|c| c.id == second));
    }
    ui.invoke_collection_action(second as i32, 3, 0);
    assert_eq!(ui.get_delete_destinations().row_count(), 0);
    ui.invoke_finish_delete(0);
    assert!(!ui.get_inline_error().as_str().is_empty());
    assert!(ui.get_collection_delete(), "失败时弹窗留在原地");
    ui.set_delete_discard(true);
    ui.invoke_finish_delete(0);
    assert!(
        ui.get_inline_error().as_str().contains("至少保留一个分组"),
        "换一种模式也不能清空最后一个分组"
    );
    {
        let s = app.borrow();
        assert!(s.collections.iter().any(|c| c.id == second));
        s.main.set_collection_delete(false);
        s.main.set_delete_discard(false);
        s.main.set_inline_error("".into());
    }
}

fn check_window_state(app: &Rc<RefCell<App>>) {
    let settings_ui = create_settings(app, &app.borrow().data_dir).unwrap();
    {
        let s = app.borrow();
        assert!(s.settings_ui.0.set(settings_ui).is_ok());
        s.sync_settings_ui();
    }
    let mut s = app.borrow_mut();
    s.settings_ui.set_keep_topmost(true);
    s.apply_settings(3);
    assert!(s.saved_window.always_on_top);
    assert!(s.wants_topmost());
    s.main.set_pinned(false);
    assert!(!s.auto_hide_guard().paused(), "always on top does not pin");
    s.save_window(true);
    assert!(s.store.window_state().unwrap().always_on_top);
    s.settings_ui.set_hotkey_text("Ctrl+Alt+Shift+F9".into());
    s.apply_settings(2);
    assert_eq!(
        s.settings.hotkey, "Ctrl+Alt+Shift+F9",
        "hotkey path remains independent"
    );
    let (_, work) = platform::geometry(s.main.window()).unwrap();
    let mut rect = platform::geometry(s.main.window()).unwrap().0;
    rect.x = work.x;
    rect.y = work.y;
    for strip in [0, 1, 6, 12] {
        s.settings.edge_hide = true;
        s.settings.edge_strip_visible = strip != 0;
        s.settings.edge_strip_width_px = strip.max(1);
        s.dock = Some((diascia::docking::Edge::Left, rect));
        s.dock_work = Some(work);
        s.main
            .window()
            .set_position(slint::PhysicalPosition::new(rect.x, rect.y));
        s.main.set_batch_mode(true);
        let now = Instant::now();
        s.apply_protection(Protection::Fullscreen, now, false);
        assert!(!s.main.window().is_visible());
        let original = s.protection_resume.0.clone();
        s.apply_protection(Protection::Strict, now, false);
        assert_eq!(
            s.protection_resume.0, original,
            "stronger protection keeps snapshot"
        );
        assert!(!s.wants_topmost(), "strict protection wins");
        s.apply_protection(Protection::Normal, now, false);
        assert!(!s.main.window().is_visible(), "release is debounced");
        s.apply_protection(Protection::Normal, now + Duration::from_secs(2), false);
        assert!(s.main.window().is_visible());
        assert!(s.edge_hidden && s.protected_edge_waiting);
        // The property is "we did not activate ourselves", so that is what gets asserted. Comparing
        // `GetForegroundWindow()` against a snapshot also fails when some unrelated app on the
        // desktop changes focus mid-test, which is not this code's doing.
        assert!(
            !platform::window_active(s.main.window()),
            "collapsed recovery never activates"
        );
        s.tick_window(false);
        assert!(
            s.edge_hidden,
            "batch interaction cannot auto expand recovery"
        );
        let actual = platform::geometry(s.main.window()).unwrap().0;
        assert_eq!(actual.x + actual.width - work.x, strip as i32);
        s.main.set_batch_mode(false);
        s.user_summon_edge();
    }
    let now = Instant::now();
    s.apply_protection(Protection::Strict, now, false);
    s.hide_all();
    s.apply_protection(Protection::Normal, now, false);
    s.apply_protection(Protection::Normal, now + Duration::from_secs(2), false);
    assert!(
        !s.main.window().is_visible(),
        "manual hide cancels recovery"
    );
    s.add_protected_app("C:\\Games\\One\\game.exe".into());
    s.add_protected_app("C:/Games/Two/game.exe".into());
    assert_eq!(
        s.settings.protected_apps,
        vec![
            "C:\\Games\\One\\game.exe".to_string(),
            "C:/Games/Two/game.exe".to_string()
        ],
        "multiple strict programs are saved"
    );
    assert_eq!(s.settings_ui.get_protected_apps().iter().count(), 2);
    s.add_protected_app("c:\\games\\one\\game.exe".into());
    assert_eq!(
        s.settings.protected_apps.len(),
        2,
        "normalized duplicate is rejected"
    );
    assert!(!s.settings_ui.get_protected_error().is_empty());
    s.remove_protected(0);
    assert_eq!(
        s.settings.protected_apps,
        vec!["C:/Games/Two/game.exe".to_string()],
        "removal targets the selected entry"
    );
    assert_eq!(s.store.settings().unwrap().protected_apps.len(), 1);
    s.add_protected_app("game.exe".into());
    assert_eq!(
        s.settings.protected_apps.len(),
        1,
        "relative path is rejected and the list rolls back"
    );
    assert!(!s.settings_ui.get_protected_error().is_empty());
    assert_eq!(s.settings_ui.get_protected_apps().iter().count(), 1);
    // Slint callbacks re-enter App, so drive them with the borrow released.
    {
        let ui = s.settings_ui.clone_strong();
        drop(s);
        let input: slint::SharedString = "C:\\Games\\Four\\game4.exe".into();
        ui.set_protected_input(input.clone());
        ui.invoke_add_protected_app(input);
        assert_eq!(
            ui.get_protected_apps().iter().count(),
            2,
            "the add callback reaches the store"
        );
        assert!(ui.get_protected_input().is_empty());
        ui.invoke_remove_protected_app(1);
        s = app.borrow_mut();
    }
    assert_eq!(
        s.settings.protected_apps,
        vec!["C:/Games/Two/game.exe".to_string()],
        "the remove callback targets the row"
    );
    s.add_protected_app("C:\\Games\\Three\\game3.exe".into());
    s.settings_ui.set_protected_error("".into());
    assert_eq!(s.settings.protected_apps.len(), 2);
    // Notices cannot be shown while protected, and must not stack into the single-row strip.
    let now = Instant::now();
    s.apply_protection(Protection::Strict, now, false);
    s.main.set_status("".into());
    for index in 0..12 {
        s.status(format!("保存失败{index}"));
    }
    assert!(
        s.main.get_status().is_empty(),
        "保护期间不向看不见的面板写字"
    );
    s.apply_protection(Protection::Normal, now, false);
    s.apply_protection(Protection::Normal, now + Duration::from_secs(2), false);
    let toast = s.main.get_status().to_string();
    assert!(toast.contains("保存失败11"), "{toast}");
    assert!(toast.contains("另有 7 条"), "{toast}");
    assert!(toast.contains("已省略 4 条"), "{toast}");
    s.apply_protection(Protection::Strict, Instant::now(), false);
    s.main.set_status("".into());
    // Manual override: the tray toggle admits a summon over a protected foreground.
    s.toggle_protection_bypass();
    assert!(s.policy.bypass());
    s.apply_protection(
        Protection::Normal,
        Instant::now() + Duration::from_secs(3),
        false,
    );
    s.show_main(ShowSource::Tray);
    assert!(
        s.main.window().is_visible(),
        "放行期间热键与托盘呼出恢复可用"
    );
    s.toggle_protection_bypass();
    assert!(!s.policy.bypass(), "放行只作用于本次操作，不写库");
    s.apply_protection(Protection::Strict, Instant::now(), false);
    assert!(!s.main.window().is_visible(), "撤销放行后整组窗口重新收起");
}
