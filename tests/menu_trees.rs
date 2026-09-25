//! Menus are data, so their contents are pinned here rather than clicked: a real popup needs a
//! native HWND, which an offscreen window never has, and driving the live launcher to inspect a row
//! costs the user their foreground.
use diascia::menus::{self, MenuContext, MenuDestination, MenuEntry, MenuItem, MenuNode, Region};

fn node(id: i32, name: &str) -> MenuNode {
    MenuNode {
        id,
        name: name.into(),
    }
}

fn destination(id: i32, name: &str, parent: i32, holds_entry: bool) -> MenuDestination {
    MenuDestination {
        id,
        name: name.into(),
        parent,
        holds_entry,
    }
}

/// Two groups, one of them holding the project this menu opens on.
fn fixture() -> MenuContext {
    MenuContext {
        target: Some(MenuEntry { id: 7, kind: 0 }),
        list_view: false,
        icon_size: 1,
        batch_blocked: false,
        can_move_up: true,
        can_move_down: true,
        groups: vec![node(1, "常用"), node(3, "工作")],
        destinations: vec![
            destination(2, "默认分类", 1, false),
            destination(4, "工具", 1, true),
            destination(5, "笔记", 3, false),
        ],
    }
}

fn walk<'a>(items: &'a [MenuItem], visit: &mut impl FnMut(&'a MenuItem)) {
    for item in items {
        visit(item);
        walk(&item.children, visit);
    }
}

fn labelled<'a>(items: &'a [MenuItem], label: &str) -> &'a MenuItem {
    let mut found = None;
    walk(items, &mut |item| {
        if item.label == label {
            found = Some(item);
        }
    });
    found.unwrap_or_else(|| panic!("菜单里没有「{label}」"))
}

fn commands(items: &[MenuItem]) -> Vec<u32> {
    let mut out = Vec::new();
    walk(items, &mut |item| {
        if !item.separator && item.children.is_empty() {
            out.push(item.command);
        }
    });
    out
}

/// Section rules may not sit at either edge or touch each other, and a submenu that opens nothing
/// would just dismiss the menu, so both shapes are rejected everywhere in the tree.
fn assert_sections_well_formed(items: &[MenuItem], kind: i32) {
    if items.is_empty() {
        return;
    }
    assert!(
        !items.first().unwrap().separator,
        "kind {kind}：分段线不能打头"
    );
    assert!(
        !items.last().unwrap().separator,
        "kind {kind}：分段线不能收尾"
    );
    for pair in items.windows(2) {
        assert!(
            !(pair[0].separator && pair[1].separator),
            "kind {kind}：两条分段线不能挨着"
        );
    }
    for item in items {
        if item.command == 0 && !item.separator {
            assert!(
                !item.children.is_empty(),
                "kind {kind}：子菜单「{}」是空的，点下去只会关掉菜单",
                item.label
            );
        }
        assert_sections_well_formed(&item.children, kind);
    }
}

#[test]
fn more_menu_carries_the_whole_blank_area_block() {
    let ctx = fixture();
    let blank = menus::tree(4, &ctx);
    let more = menus::tree(0, &ctx);
    assert!(!blank.is_empty());
    assert_eq!(
        &more[..blank.len()],
        &blank[..],
        "标题栏「更多」必须原样带上空白处菜单的每一行"
    );
}

#[test]
fn every_kind_is_segmented_without_dead_rows() {
    let ctx = fixture();
    for kind in 0..=4 {
        let items = menus::tree(kind, &ctx);
        assert!(!items.is_empty(), "kind {kind} 应该有菜单");
        assert!(
            items.iter().any(|item| item.separator),
            "kind {kind} 应该分段"
        );
        assert_sections_well_formed(&items, kind);
    }
}

#[test]
fn every_row_number_is_routed_exactly_once() {
    let ctx = fixture();
    for kind in 0..=4 {
        let items = menus::tree(kind, &ctx);
        let mut seen = commands(&items);
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "kind {kind}：命令号不能重复");
        for command in commands(&items) {
            assert_ne!(command, 0, "kind {kind}：可点击行必须有命令号");
            assert!(
                menus::region(kind, command).is_some(),
                "kind {kind} 命令 {command} 没有归属，点了会被静默吞掉"
            );
        }
    }
}

#[test]
fn the_manage_block_is_the_only_one_shared_by_two_menus() {
    for command in [
        diascia::menus::BATCH_BASE,
        diascia::menus::VIEW_BASE,
        diascia::menus::VIEW_BASE + 6,
    ] {
        assert_eq!(menus::region(0, command), Some(Region::Manage));
        assert_eq!(menus::region(4, command), Some(Region::Manage));
    }
    // The blank-area menu has nothing to route the application commands to.
    assert_eq!(menus::region(4, 9), None);
    assert_eq!(menus::region(0, 9), Some(Region::App));
}

#[test]
fn no_row_ships_the_placeholder_glyph() {
    let ctx = fixture();
    for kind in 0..=4 {
        let items = menus::tree(kind, &ctx);
        walk(&items, &mut |item| {
            if !item.separator {
                assert_ne!(
                    item.icon,
                    diascia::menus::DEFAULT_ICON,
                    "kind {kind}「{}」还在用 action() 的默认字形（一张人像）",
                    item.label
                );
            }
        });
    }
    assert_ne!(
        labelled(&menus::tree(0, &ctx), "备份…").icon,
        labelled(&menus::tree(1, &ctx), "删除…").icon,
        "备份和删除不能是同一个字形"
    );
    for kind in [1, 2, 3] {
        assert_eq!(
            labelled(&menus::tree(kind, &ctx), "删除…").icon,
            '\u{e74d}',
            "kind {kind} 的删除必须与项目菜单同一个垃圾桶字形"
        );
    }
}

#[test]
fn file_only_rows_go_grey_over_a_url() {
    let mut ctx = fixture();
    ctx.target = Some(MenuEntry { id: 7, kind: 3 });
    let url = menus::tree(1, &ctx);
    for label in [
        "打开所在位置",
        "重新定位…",
        "Windows 原生菜单",
        "转为相对路径",
        "转为绝对路径",
        "使用管理员打开",
    ] {
        assert!(
            !labelled(&url, label).enabled,
            "网址没有本地路径，「{label}」必须置灰"
        );
    }
    assert!(labelled(&url, "打开").enabled);
    for kind in [0, 4] {
        ctx.target = Some(MenuEntry { id: 7, kind });
        assert!(
            labelled(&menus::tree(1, &ctx), "使用管理员打开").enabled,
            "kind {kind} 可以提权运行"
        );
    }
    ctx.target = Some(MenuEntry { id: 7, kind: 5 });
    assert!(
        !labelled(&menus::tree(1, &ctx), "使用管理员打开").enabled,
        "打包应用不走 ShellExecute 的 runas"
    );
}

#[test]
fn sorting_refuses_to_move_past_either_end() {
    for kind in [1, 2, 3] {
        let mut ctx = fixture();
        ctx.can_move_up = false;
        ctx.can_move_down = false;
        let items = menus::tree(kind, &ctx);
        assert!(
            !labelled(&items, "前移").enabled,
            "kind {kind} 到头了还要前移就是空转"
        );
        assert!(!labelled(&items, "后移").enabled, "kind {kind} 到底了");
        ctx.can_move_up = true;
        ctx.can_move_down = false;
        let items = menus::tree(kind, &ctx);
        assert!(labelled(&items, "前移").enabled);
        assert!(!labelled(&items, "后移").enabled);
    }
}

#[test]
fn the_last_group_cannot_be_deleted() {
    let mut ctx = fixture();
    ctx.groups = vec![node(1, "常用")];
    assert!(
        !labelled(&menus::tree(2, &ctx), "删除…").enabled,
        "只剩一个分组时不能删"
    );
    assert!(
        labelled(&menus::tree(3, &ctx), "删除…").enabled,
        "删子分类不受分组数量限制"
    );
    assert!(labelled(&menus::tree(2, &ctx), "新建分组").enabled);
}

#[test]
fn a_destination_holding_the_entry_cannot_take_it_again() {
    let items = menus::tree(1, &fixture());
    let move_to = labelled(&items, "移动到");
    let copy_to = labelled(&items, "复制到");
    for (menu, base) in [
        (move_to, diascia::menus::MOVE_BASE),
        (copy_to, diascia::menus::COPY_BASE),
    ] {
        let tools = menu
            .children
            .iter()
            .flat_map(|group| group.children.iter())
            .find(|row| row.label == "工具")
            .expect("常用 / 工具");
        assert_eq!(tools.command, base + 1, "命令号就是 destinations 下标");
        assert!(!tools.enabled, "已经在「工具」里就不能再移过去");
        let notes = menu
            .children
            .iter()
            .flat_map(|group| group.children.iter())
            .find(|row| row.label == "笔记")
            .expect("工作 / 笔记");
        assert!(notes.enabled);
    }
}

#[test]
fn a_group_without_destinations_gets_no_row() {
    let mut ctx = fixture();
    ctx.destinations.retain(|d| d.parent != 3);
    let items = menus::tree(1, &ctx);
    let move_to = labelled(&items, "移动到");
    assert_eq!(move_to.children.len(), 1, "空分组不该占一行");
    assert_eq!(move_to.children[0].label, "常用");
}

#[test]
fn view_rows_reflect_the_current_view() {
    let mut ctx = fixture();
    for kind in [0, 4] {
        let items = menus::tree(kind, &ctx);
        assert_eq!(labelled(&items, "中图标 · 四列").icon, '\u{e73e}');
        assert_eq!(labelled(&items, "小图标 · 五列").icon, '\u{e80a}');
        assert_eq!(labelled(&items, "列表查看").icon, '\u{e8fd}');
        assert!(labelled(&items, "多选整理").enabled);
        ctx.list_view = true;
        ctx.icon_size = 0;
        let items = menus::tree(kind, &ctx);
        assert_eq!(
            labelled(&items, "小图标 · 五列").icon,
            '\u{e80a}',
            "列表视图下没有网格密度可言"
        );
        assert_eq!(labelled(&items, "列表查看").icon, '\u{e73e}');
        ctx.list_view = false;
        ctx.icon_size = 1;
    }
    ctx.batch_blocked = true;
    assert!(
        !labelled(&menus::tree(4, &ctx), "多选整理").enabled,
        "编辑中不能进多选"
    );
}

#[test]
fn unrelated_rows_of_one_menu_never_share_a_glyph() {
    // Related operations may share a family on purpose (复制路径 and 复制到 are both copies), but two
    // unrelated rows in one popup wearing the same glyph read as one operation. 数据 once wore
    // 备份's archive box, and 多选整理 wore 列表查看's list.
    let ctx = fixture();
    let more = menus::tree(0, &ctx);
    let item = menus::tree(1, &ctx);
    for (menu, left, right) in [
        (&more, "数据", "备份…"),
        (&more, "多选整理", "列表查看"),
        (&more, "路径转换", "图标大小"),
        (&item, "移动到", "复制到"),
        (&item, "删除…", "编辑…"),
    ] {
        assert_ne!(
            labelled(menu, left).icon,
            labelled(menu, right).icon,
            "「{left}」和「{right}」不能长得一样"
        );
    }
}

#[test]
fn unclaimed_kinds_open_nothing() {
    let ctx = fixture();
    for kind in [5, 6, -1, i32::MAX] {
        assert!(menus::tree(kind, &ctx).is_empty(), "kind {kind} 已废弃");
        assert!(menus::region(kind, 1).is_none());
    }
    // The left rail's blank area lost its menu on purpose: both navigation gutters behave alike.
    assert!(labelled(&menus::tree(3, &ctx), "新建子分类").enabled);
}

#[test]
fn the_interface_only_opens_kinds_that_build_a_tree() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/app.slint"))
        .expect("app.slint");
    // The `callback native-menu(int, int)` declaration is deliberately not a match here.
    let marker = "root.native-menu(";
    let mut kinds = std::collections::BTreeSet::new();
    let mut emitters = 0;
    for (offset, matched) in source.match_indices(marker) {
        emitters += 1;
        let digits: String = source[offset + matched.len()..]
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .map(|byte| byte as char)
            .collect();
        kinds.insert(
            digits.parse::<i32>().unwrap_or_else(|error| {
                panic!("native-menu 的第一参数必须是 kind 字面量（{error}）")
            }),
        );
    }
    assert_eq!(emitters, 6, ".slint 里的 native-menu 调用点数量变了");
    let ctx = fixture();
    assert_eq!(
        kinds,
        [0, 1, 2, 3, 4].into_iter().collect(),
        ".slint 发出的 kind 必须和 menus::tree 覆盖的完全一致"
    );
    for kind in kinds {
        assert!(!menus::tree(kind, &ctx).is_empty());
    }
}
