//! Owned Fluent popup trees; native tracking keeps keyboard and monitor placement reliable.
//!
//! Trees are built here, from plain data, so their contents are provable without a window: a real
//! popup needs an HWND (`fluent_menu::show` bails when `native_handle` returns `None`), so this is
//! the only place the menu can be inspected by a test.
#![allow(clippy::upper_case_acronyms)]

/// A context menu row that shows a contact card is a bug, not a default: every action must name
/// its own glyph. Kept as a constant so `tests/menu_trees.rs` can forbid it.
pub const DEFAULT_ICON: char = '\u{e8a5}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub command: u32,
    pub enabled: bool,
    pub children: Vec<MenuItem>,
    pub icon: char,
    /// A thin rule between sections. Drawn by us rather than `MFT_SEPARATOR` so it follows the
    /// Fluent palette, and kept disabled so menu tracking skips it.
    pub separator: bool,
}
impl MenuItem {
    pub fn action(label: impl Into<String>, command: u32) -> Self {
        Self {
            label: label.into(),
            command,
            enabled: true,
            children: vec![],
            icon: DEFAULT_ICON,
            separator: false,
        }
    }
    pub fn submenu(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            command: 0,
            enabled: true,
            children,
            icon: '\u{e8b7}',
            separator: false,
        }
    }
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            command: 0,
            enabled: false,
            children: vec![],
            icon: ' ',
            separator: true,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }
    pub fn icon(mut self, icon: char) -> Self {
        self.icon = icon;
        self
    }
}

/// A group, or a group-level row in a cascade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub id: i32,
    pub name: String,
}

/// One selectable destination. `id` order is the command numbering for 移动到/复制到, so the caller
/// must hand these over in the same order it publishes them as `destinations`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuDestination {
    pub id: i32,
    pub name: String,
    pub parent: i32,
    /// The entry this menu is about already lives here, so moving or copying to it is a no-op.
    pub holds_entry: bool,
}

/// The project a context menu was opened on. `kind` is `Target::kind_index`: 0 program, 1 file,
/// 2 folder, 3 url, 4 shortcut, 5 packaged app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: i32,
    pub kind: i32,
}

/// Everything a tree needs, read off the UI before the menu opens. Deliberately free of Slint
/// types: the binary snapshots its state into this, then holds no borrow while the popup is built.
#[derive(Clone, Debug, Default)]
pub struct MenuContext {
    pub target: Option<MenuEntry>,
    pub list_view: bool,
    pub icon_size: i32,
    /// An editor session, an import or a file dialog is open, so batch mode cannot start.
    pub batch_blocked: bool,
    /// Boundaries mirroring `Store::move_entry` for the row this menu targets: the item menu
    /// resolves them against same-collection siblings, the collection menu against siblings under
    /// the same parent. Moving past either end is a silent no-op in storage, so the menu says no.
    pub can_move_up: bool,
    pub can_move_down: bool,
    pub groups: Vec<MenuNode>,
    pub destinations: Vec<MenuDestination>,
}

/// Command bases. `region` and `tree` are the only places allowed to spell these out.
pub const BATCH_BASE: u32 = 30;
pub const VIEW_BASE: u32 = 40;
pub const MOVE_BASE: u32 = 100;
pub const COPY_BASE: u32 = 10_000;

/// Which dispatch arm of `ui_commands` handles a command. A tree may only emit ids its kind's
/// region accepts; the test over both directions is what keeps the two menus built here from
/// drifting away from the router in the binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// Shared with the empty-space menu: batch mode, add, view and sort.
    Manage,
    /// Whole-application commands that only the title-bar 更多 menu offers.
    App,
    /// Commands about one project.
    Item,
    /// Commands about a group or category.
    Collection,
}

pub fn region(kind: i32, command: u32) -> Option<Region> {
    match kind {
        0 | 4 => {
            if command == BATCH_BASE || (VIEW_BASE..VIEW_BASE + 8).contains(&command) {
                Some(Region::Manage)
            } else if kind == 0 && (1..=9).contains(&command) {
                Some(Region::App)
            } else {
                None
            }
        }
        1 => ((1..=4).contains(&command) || (6..=15).contains(&command) || command >= MOVE_BASE)
            .then_some(Region::Item),
        2 | 3 => ((1..=6).contains(&command) || command >= MOVE_BASE).then_some(Region::Collection),
        _ => None,
    }
}

/// The popup tree for `kind`: 0 title-bar 更多, 1 project, 2 group, 3 category, 4 empty project
/// area. Any other kind opens nothing.
pub fn tree(kind: i32, ctx: &MenuContext) -> Vec<MenuItem> {
    tree_for_locale(kind, ctx, crate::i18n::locale())
}

pub fn tree_for_locale(kind: i32, ctx: &MenuContext, locale: crate::i18n::Locale) -> Vec<MenuItem> {
    let s = |zh, en| crate::i18n::t_for(locale, zh, en);
    match kind {
        // At 440 logical pixels wide the grid leaves very little blank edge to hit, so everything
        // the empty-space menu offers is repeated here.
        0 => {
            let mut items = manage_menu(ctx, locale);
            items.extend([
                MenuItem::separator(),
                MenuItem::submenu(
                    s("数据", "Data"),
                    vec![
                        MenuItem::action(s("导出…", "Export…"), 1).icon('\u{e74e}'),
                        MenuItem::action(s("恢复…", "Restore…"), 2).icon('\u{e8e5}'),
                        MenuItem::separator(),
                        MenuItem::action(s("检查失效项目…", "Check Broken Items…"), 3)
                            .icon('\u{e9ce}'),
                        MenuItem::action(s("打开数据目录", "Open Data Folder"), 4).icon('\u{e8b7}'),
                    ],
                )
                .icon('\u{e81e}'),
                MenuItem::submenu(
                    s("路径转换", "Convert Paths"),
                    vec![
                        MenuItem::action(s("转换为相对路径…", "Convert to Relative Paths…"), 6)
                            .icon('\u{e8ab}'),
                        MenuItem::action(s("转换为绝对路径…", "Convert to Absolute Paths…"), 7)
                            .icon('\u{e8ab}'),
                    ],
                )
                .icon('\u{e8ab}'),
                MenuItem::separator(),
                MenuItem::action(s("恢复窗口位置", "Restore Window Position"), 5).icon('\u{e8a7}'),
                MenuItem::action(s("设置…", "Settings…"), 8).icon('\u{e713}'),
                MenuItem::separator(),
                MenuItem::action(s("退出", "Quit"), 9).icon('\u{f3b1}'),
            ]);
            items
        }
        1 => item_menu(ctx, locale),
        2 | 3 => collection_menu(ctx, kind, locale),
        4 => manage_menu(ctx, locale),
        _ => vec![],
    }
}

/// Commands 30 and 40..=46: the list-management block. Both the empty-space menu and the title-bar
/// 更多 menu open with these items — at 440 logical pixels the grid rarely leaves a blank edge to
/// right-click — so one builder and one router keep the two menus from drifting apart.
fn manage_menu(ctx: &MenuContext, locale: crate::i18n::Locale) -> Vec<MenuItem> {
    let s = |zh, en| crate::i18n::t_for(locale, zh, en);
    let check = |active: bool| if active { '\u{e73e}' } else { '\u{e80a}' };
    let grid = !ctx.list_view;
    vec![
        MenuItem::action(s("多选整理", "Select Multiple"), BATCH_BASE)
            .icon('\u{e73a}')
            .disabled(ctx.batch_blocked),
        MenuItem::separator(),
        MenuItem::action(s("添加项目…", "Add Item…"), VIEW_BASE).icon('\u{e710}'),
        MenuItem::action(s("添加文件…", "Add Files…"), VIEW_BASE + 1).icon('\u{e8b7}'),
        MenuItem::action(s("扫描导入软件…", "Scan Installed Apps…"), VIEW_BASE + 7)
            .icon('\u{e773}'),
        MenuItem::separator(),
        MenuItem::submenu(
            s("图标大小", "Icon Size"),
            vec![
                MenuItem::action(s("小图标 · 五列", "Small · 5 Columns"), VIEW_BASE + 2)
                    .icon(check(grid && ctx.icon_size == 0)),
                MenuItem::action(s("中图标 · 四列", "Medium · 4 Columns"), VIEW_BASE + 3)
                    .icon(check(grid && ctx.icon_size == 1)),
                MenuItem::action(s("大图标 · 三列", "Large · 3 Columns"), VIEW_BASE + 4)
                    .icon(check(grid && ctx.icon_size == 2)),
            ],
        )
        .icon('\u{e80a}'),
        MenuItem::action(s("列表查看", "List View"), VIEW_BASE + 5).icon(if ctx.list_view {
            '\u{e73e}'
        } else {
            '\u{e8fd}'
        }),
        MenuItem::action(s("按名称排序", "Sort by Name"), VIEW_BASE + 6).icon('\u{e8cb}'),
    ]
}

/// Destinations under `parent`, numbered by their index in `ctx.destinations` so the binary can
/// resolve a command back to the same row it published. A group with nothing in it gets no row at
/// all: an owner-draw submenu with no children would just close the menu.
fn cascade(
    ctx: &MenuContext,
    label: &str,
    icon: char,
    base: u32,
    disabled: impl Fn(&MenuDestination) -> bool,
) -> MenuItem {
    let groups: Vec<_> = ctx
        .groups
        .iter()
        .map(|group| {
            let children: Vec<_> = ctx
                .destinations
                .iter()
                .enumerate()
                .filter(|(_, d)| d.parent == group.id)
                .map(|(index, d)| {
                    MenuItem::action(d.name.clone(), base + index as u32)
                        // A destination is a name to pick, not an operation: the gutter stays
                        // empty rather than repeating a folder glyph down the whole column.
                        .icon(' ')
                        .disabled(disabled(d))
                })
                .collect();
            MenuItem::submenu(group.name.clone(), children)
        })
        .filter(|item| !item.children.is_empty())
        .collect();
    MenuItem::submenu(label, groups).icon(icon)
}

fn item_menu(ctx: &MenuContext, locale: crate::i18n::Locale) -> Vec<MenuItem> {
    let s = |zh, en| crate::i18n::t_for(locale, zh, en);
    let file = ctx
        .target
        .as_ref()
        .is_some_and(|entry| entry.kind != 3 && entry.kind != 5);
    let run_as_admin = ctx
        .target
        .as_ref()
        .is_some_and(|entry| entry.kind == 0 || entry.kind == 4);
    vec![
        MenuItem::action(s("打开", "Open"), 1).icon('\u{e768}'),
        MenuItem::action(s("打开所在位置", "Open File Location"), 2)
            .icon('\u{e8b7}')
            .disabled(!file),
        MenuItem::action(s("复制路径 / 网址", "Copy Path / URL"), 3).icon('\u{e8c8}'),
        MenuItem::separator(),
        MenuItem::action(s("编辑…", "Edit…"), 4).icon('\u{e70f}'),
        MenuItem::action(s("重新定位…", "Relocate…"), 11)
            .icon('\u{e8b7}')
            .disabled(!file),
        MenuItem::separator(),
        MenuItem::action(s("使用管理员打开", "Run as Administrator"), 12)
            .icon('\u{e7ef}')
            .disabled(!run_as_admin),
        MenuItem::action(s("Windows 原生菜单", "Windows Context Menu"), 13)
            .icon('\u{e700}')
            .disabled(!file),
        MenuItem::action(s("创建桌面快捷方式", "Create Desktop Shortcut"), 14).icon('\u{e8a7}'),
        MenuItem::action(s("刷新图标缓存", "Refresh Icon"), 15).icon('\u{e72c}'),
        MenuItem::separator(),
        cascade(ctx, s("移动到", "Move to"), '\u{e8de}', MOVE_BASE, |d| {
            d.holds_entry
        }),
        cascade(ctx, s("复制到", "Copy to"), '\u{e8c8}', COPY_BASE, |d| {
            d.holds_entry
        }),
        MenuItem::separator(),
        MenuItem::submenu(
            s("排序", "Reorder"),
            vec![
                MenuItem::action(s("前移", "Move Earlier"), 8)
                    .icon('\u{e70e}')
                    .disabled(!ctx.can_move_up),
                MenuItem::action(s("后移", "Move Later"), 9)
                    .icon('\u{e70d}')
                    .disabled(!ctx.can_move_down),
            ],
        ),
        MenuItem::submenu(
            s("路径转换", "Convert Path"),
            vec![
                MenuItem::action(s("转为相对路径", "Make Relative"), 6)
                    .icon('\u{e8ab}')
                    .disabled(!file),
                MenuItem::action(s("转为绝对路径", "Make Absolute"), 7)
                    .icon('\u{e8ab}')
                    .disabled(!file),
            ],
        ),
        MenuItem::separator(),
        MenuItem::action(s("删除…", "Delete…"), 10).icon('\u{e74d}'),
    ]
}

fn collection_menu(ctx: &MenuContext, kind: i32, locale: crate::i18n::Locale) -> Vec<MenuItem> {
    let s = |zh, en| crate::i18n::t_for(locale, zh, en);
    let mut items = vec![
        MenuItem::action(
            if kind == 2 {
                s("新建分组", "New Group")
            } else {
                s("新建子分类", "New Category")
            },
            1,
        )
        .icon('\u{e710}'),
        MenuItem::action(s("编辑…", "Rename…"), 3).icon('\u{e70f}'),
        MenuItem::separator(),
        MenuItem::submenu(
            s("排序", "Reorder"),
            vec![
                MenuItem::action(s("前移", "Move Earlier"), 4)
                    .icon('\u{e70e}')
                    .disabled(!ctx.can_move_up),
                MenuItem::action(s("后移", "Move Later"), 5)
                    .icon('\u{e70d}')
                    .disabled(!ctx.can_move_down),
            ],
        ),
    ];
    if kind == 3 {
        let groups = ctx
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                MenuItem::action(group.name.clone(), MOVE_BASE + index as u32).icon(' ')
            })
            .collect();
        items.push(MenuItem::submenu(s("移动到分组", "Move to Group"), groups).icon('\u{e8de}'));
    }
    items.extend([
        MenuItem::separator(),
        MenuItem::action(s("删除…", "Delete…"), 6)
            .icon('\u{e74d}')
            .disabled(kind == 2 && ctx.groups.len() == 1),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk<'a>(items: &'a [MenuItem], visit: &mut impl FnMut(&'a MenuItem)) {
        for item in items {
            visit(item);
            walk(&item.children, visit);
        }
    }

    #[test]
    fn actionable_commands_are_unique_and_routed() {
        let ctx = MenuContext {
            target: Some(MenuEntry { id: 7, kind: 0 }),
            groups: vec![
                MenuNode {
                    id: 1,
                    name: "常用".into(),
                },
                MenuNode {
                    id: 3,
                    name: "工作".into(),
                },
            ],
            destinations: vec![
                MenuDestination {
                    id: 2,
                    name: "默认分类".into(),
                    parent: 1,
                    holds_entry: false,
                },
                MenuDestination {
                    id: 4,
                    name: "工具".into(),
                    parent: 1,
                    holds_entry: true,
                },
            ],
            ..Default::default()
        };
        for locale in [crate::i18n::Locale::ZhCn, crate::i18n::Locale::EnUs] {
            for kind in 0..=4 {
                let items = tree_for_locale(kind, &ctx, locale);
                let mut seen = Vec::new();
                walk(&items, &mut |item| {
                    if !item.separator && item.children.is_empty() {
                        assert_ne!(item.command, 0);
                        assert!(region(kind, item.command).is_some());
                        seen.push(item.command);
                    }
                    if locale == crate::i18n::Locale::EnUs
                        && item.command < MOVE_BASE
                        && !item.separator
                        && !matches!(item.label.as_str(), "常用" | "工作" | "默认分类" | "工具")
                    {
                        assert!(
                            !item
                                .label
                                .chars()
                                .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                            "unexpected CJK in English menu: {}",
                            item.label
                        );
                    }
                });
                let total = seen.len();
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(seen.len(), total);
            }
        }
    }
}
