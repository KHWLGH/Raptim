use crate::PlatformHandle;
use crate::demo::PreviewData;
use raptim::{EditorWindow, I18n, Item, MainWindow, SettingsWindow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub struct SceneSpec {
    pub id: &'static str,
    pub group: &'static str,
    pub group_en: &'static str,
    pub title: &'static str,
    pub title_en: &'static str,
    pub description: &'static str,
    pub description_en: &'static str,
    pub in_showcase: bool,
    pub width: f32,
    pub height: f32,
}

impl SceneSpec {
    pub fn group_for(self, en: bool) -> &'static str {
        if en { self.group_en } else { self.group }
    }

    pub fn title_for(self, en: bool) -> &'static str {
        if en { self.title_en } else { self.title }
    }

    pub fn description_for(self, en: bool) -> &'static str {
        if en {
            self.description_en
        } else {
            self.description
        }
    }
}

pub const SCENES: &[SceneSpec] = &[
    SceneSpec {
        id: "main-grid",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 网格视图",
        title_en: "Main Window · Grid View",
        description: "包含横向分组栏、左侧分类导航与多列项目网格，支持图标缩放与键盘焦点定位。",
        description_en: "Top group tabs, left category sidebar, and multi-column item grid with icon sizing and keyboard navigation.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-list",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 列表视图",
        title_en: "Main Window · List View",
        description: "单列列表视图，展示项目名称与完整路径/应用标识，同时演示标题栏图钉固定状态。",
        description_en: "Single-column list view displaying item names, full paths/AUMIDs, and the pinned title-bar state.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-search",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 搜索浮层",
        title_en: "Main Window · Live Search",
        description: "支持按全部/当前分组/当前分类过滤，结合中文全拼、首字母与备注快速定位项目。",
        description_en: "Scoped filtering across All/Group/Category with instant matching over names, aliases, Pinyin, and notes.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-batch",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 多选整理",
        title_en: "Main Window · Batch Selection",
        description: "勾选多个项目后在底部浮现三等分操作面板，支持批量移动、复制或移除记录。",
        description_en: "Select multiple items to open the bottom action bar for batch move, copy, or removal.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-batch-target",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 批量目标选择",
        title_en: "Main Window · Batch Destination Picker",
        description: "批量移动或复制时展开分组/分类树形选择器，支持键盘导航与重复目标预检。",
        description_en: "Expandable group/category tree picker with keyboard navigation and duplicate clash pre-check.",
        in_showcase: false,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-discover",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 扫描导入软件",
        title_en: "Main Window · Scan & Import Apps",
        description: "后台扫描系统开始菜单与已安装应用列表，支持关键词筛选并勾选批量导入。",
        description_en: "Background scan of Start Menu shortcuts and installed Store/UWP apps with filter and batch import.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-broken",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 失效项目检查",
        title_en: "Main Window · Broken Item Check",
        description: "巡检本地路径与已安装应用有效性，针对异常条目提供重定位、编辑或移除入口。",
        description_en: "Audit local paths and installed apps and relocate, edit, or remove missing targets in one place.",
        in_showcase: true,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-convert",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 批量路径转换",
        title_en: "Main Window · Batch Path Conversion",
        description: "支持三态勾选分组/分类树与转换预览，以数据目录为基准批量切换相对或绝对路径。",
        description_en: "Tri-state scope tree and conversion preview to batch-toggle relative and absolute paths.",
        in_showcase: false,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-empty",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 空分类引导",
        title_en: "Main Window · Empty Category",
        description: "空分类下提供添加项目、扫描导入与拖入提示，降低新用户的初次配置门槛。",
        description_en: "Onboarding actions for empty categories: add items, scan installed apps, or drop files and URLs.",
        in_showcase: false,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-rename",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 窗内名称编辑",
        title_en: "Main Window · Inline Rename",
        description: "新建分组、新建子分类或重命名时弹出居中浮层输入框，避免额外窗口切换。",
        description_en: "Centered inline prompt for creating groups/categories or renaming without extra dialogs.",
        in_showcase: false,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "main-delete",
        group: "主窗口",
        group_en: "Main Window",
        title: "主窗口 · 分类删除与迁移",
        title_en: "Main Window · Delete & Migrate Category",
        description: "删除非空分类时可选择将原有项目与子分类安全迁移到其他位置，或直接清理。",
        description_en: "Safely migrate items and subcategories to another destination before deleting a container.",
        in_showcase: false,
        width: 440.0,
        height: 640.0,
    },
    SceneSpec {
        id: "editor-program",
        group: "编辑窗口",
        group_en: "Editor",
        title: "编辑项目 · 本地程序",
        title_en: "Editor · Local Program",
        description: "支持配置程序路径、启动参数、工作目录、管理员提权、打开方式、别名与备注。",
        description_en: "Configure executable path, arguments, working directory, elevation, run mode, aliases, and notes.",
        in_showcase: true,
        width: 520.0,
        height: 650.0,
    },
    SceneSpec {
        id: "editor-url",
        group: "编辑窗口",
        group_en: "Editor",
        title: "编辑项目 · 网页地址",
        title_en: "Editor · Web URL",
        description: "针对网址类型自动精简表单字段，支持抓取网站图标与省略协议自动补全。",
        description_en: "Streamlined URL form with automatic HTTPS completion and on-demand favicon fetching.",
        in_showcase: false,
        width: 520.0,
        height: 650.0,
    },
    SceneSpec {
        id: "editor-app-picker",
        group: "编辑窗口",
        group_en: "Editor",
        title: "编辑项目 · 已安装应用选择",
        title_en: "Editor · Installed App Picker",
        description: "在编辑窗口内直接搜索并回填本机 Store / UWP 应用 AUMID，无需手动记忆标识。",
        description_en: "Search and select Store/UWP applications inside the editor to populate their AUMID.",
        in_showcase: false,
        width: 520.0,
        height: 650.0,
    },
    SceneSpec {
        id: "editor-collection-tree",
        group: "编辑窗口",
        group_en: "Editor",
        title: "编辑项目 · 所属分类树",
        title_en: "Editor · Category Tree Picker",
        description: "通过可折叠分组树为项目选择归属分类，键盘方向键与回车即可快速确认。",
        description_en: "Collapsible hierarchy picker with arrow-key and Enter navigation for assigning item category.",
        in_showcase: false,
        width: 520.0,
        height: 650.0,
    },
    SceneSpec {
        id: "settings-general",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 常规",
        title_en: "Settings · General",
        description: "配置浅深色外观主题、界面语言、开机静默启动、打开后隐藏、启动方式与悬停切换行为。",
        description_en: "Configure theme, UI language, autostart, hide-after-launch, click mode, and hover switching.",
        in_showcase: true,
        width: 660.0,
        height: 480.0,
    },
    SceneSpec {
        id: "settings-summon",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 呼出与隐藏",
        title_en: "Settings · Summon & Hide",
        description: "录制全局呼出热键，调节贴边自动收纳、1–12 px 露边宽度、收起延迟与始终置顶。",
        description_en: "Record global hotkey and tune edge auto-hide, 1–12 px edge strip width, collapse delay, and topmost.",
        in_showcase: true,
        width: 660.0,
        height: 480.0,
    },
    SceneSpec {
        id: "settings-protection",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 游戏勿扰",
        title_en: "Settings · Game DND",
        description: "管理全屏保护、临时旁路开关、脱敏判定日志与最多 100 项严格保护程序列表。",
        description_en: "Manage fullscreen protection, session bypass, redacted decision logs, and up to 100 strict-protected apps.",
        in_showcase: true,
        width: 660.0,
        height: 480.0,
    },
    SceneSpec {
        id: "settings-data",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 数据",
        title_en: "Settings · Data",
        description: "查看便携数据目录、切换相对路径偏好，以及执行 JSON 导出、恢复与打开目录。",
        description_en: "Inspect data folder, toggle relative-path preference, and run JSON export/restore.",
        in_showcase: false,
        width: 660.0,
        height: 480.0,
    },
    SceneSpec {
        id: "settings-about",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 关于",
        title_en: "Settings · About",
        description: "展示应用版本、Rust/Slint/SQLite 构建栈、GPL-3.0-only 许可声明与归属卡片。",
        description_en: "Show version, Rust/Slint/SQLite stack, GPL-3.0-only license, and attribution links.",
        in_showcase: true,
        width: 660.0,
        height: 480.0,
    },
    SceneSpec {
        id: "settings-notices",
        group: "设置窗口",
        group_en: "Settings",
        title: "设置 · 第三方组件说明",
        title_en: "Settings · Third-Party Notices",
        description: "窗内离线浏览所有直接依赖 crate 的版本、开源协议与用途说明。",
        description_en: "Inspect versions, licenses, and roles of all direct crate dependencies offline.",
        in_showcase: false,
        width: 660.0,
        height: 480.0,
    },
];

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(items)).into()
}

fn strings(values: &[String]) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = values
        .iter()
        .map(|s| SharedString::from(s.as_str()))
        .collect();
    Rc::new(VecModel::from(items)).into()
}

fn configure_main(ui: &MainWindow, data: &PreviewData, batch_checked: bool, en: bool) {
    ui.global::<I18n>().set_en(en);
    ui.set_window_active(true);
    ui.set_groups(model(data.groups.clone()));
    ui.set_categories(model(data.categories.clone()));
    ui.set_destinations(model(data.destinations.clone()));
    ui.set_group_id(data.active_group);
    ui.set_category_id(data.active_category);
    ui.set_icon_size(1);
    ui.set_list_view(false);
    let items: Vec<Item> = data
        .grid_items
        .iter()
        .cloned()
        .map(|mut item| {
            if !batch_checked {
                item.checked = false;
            }
            item
        })
        .collect();
    ui.set_items(model(items));
}

fn configure_settings(ui: &SettingsWindow, data: &PreviewData, dark: bool, en: bool) {
    ui.global::<I18n>().set_en(en);
    ui.set_window_active(true);
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    ui.set_rust_version(env!("RAPTIM_RUST_VERSION").into());
    ui.set_theme_mode(if dark { 2 } else { 1 });
    ui.set_language_mode(if en { 2 } else { 1 });
    ui.set_data_path(data.data_path.clone().into());
    ui.set_license_lines(strings(&data.license_lines));
    ui.set_notice_items(model(data.notice_items.clone()));
    ui.set_protected_apps(strings(&data.protected_apps));
}

/// Renders one scene offscreen via Slint's software renderer without creating an OS window.
pub fn render_scene(
    scene: &SceneSpec,
    data: &PreviewData,
    dark: bool,
    scale: f32,
    en: bool,
    platform: &PlatformHandle,
) -> Result<image::RgbImage, String> {
    match scene.id {
        "main-grid" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            if let Some(first) = data.grid_items.first() {
                ui.set_selected_id(first.id);
                ui.set_keyboard_focus(true);
            }
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-list" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_list_view(true);
            ui.set_pinned(true);
            if let Some(first) = data.list_items.first() {
                ui.set_selected_id(first.id);
            }
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-search" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_search_open(true);
            ui.set_search_scope(0);
            ui.set_query(data.search_query.clone().into());
            ui.set_results(model(data.search_results.clone()));
            if let Some(first) = data.search_results.first() {
                ui.set_selected_id(first.id);
            }
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-batch" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, true, en);
            let checked_count = data.grid_items.iter().filter(|i| i.checked).count() as i32;
            ui.set_batch_mode(true);
            ui.set_batch_count(checked_count);
            ui.set_batch_all_selected(
                checked_count > 0 && checked_count == data.grid_items.len() as i32,
            );
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-batch-target" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, true, en);
            let checked_count = data.grid_items.iter().filter(|i| i.checked).count() as i32;
            ui.set_batch_mode(true);
            ui.set_batch_count(checked_count.max(1));
            ui.set_batch_target_open(true);
            ui.set_batch_action(0);
            ui.set_batch_nodes(model(data.batch_nodes.clone()));
            ui.set_batch_destination(data.active_category);
            ui.set_batch_tree_index(1);
            ui.set_batch_target_name(
                if en {
                    "Destination: Common / Daily Apps"
                } else {
                    "目标位置：常用 / 日常应用"
                }
                .into(),
            );
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-discover" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            let selected = data.discover_items.iter().filter(|d| d.checked).count() as i32;
            ui.set_discover_open(true);
            ui.set_discover_loading(false);
            ui.set_discover_target_name(
                if en {
                    "Common / Daily Apps"
                } else {
                    "常用 / 日常应用"
                }
                .into(),
            );
            ui.set_discover_items(model(data.discover_items.clone()));
            ui.set_discover_selected_count(selected);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-broken" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_broken_open(true);
            ui.set_broken_loading(false);
            ui.set_broken_items(model(data.broken_items.clone()));
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-convert" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_conversion_open(true);
            ui.set_conversion_relative(true);
            ui.set_scopes(model(data.conversion_scopes.clone()));
            ui.set_scope_selected(true);
            ui.set_conversion_summary(
                if en {
                    "14 convertible paths (2 cross-drive skipped, 3 URL/app entries N/A)"
                } else {
                    "可转换 14 项路径（跳过 2 项跨驱动器，3 项网页/应用不适用）"
                }
                .into(),
            );
            ui.set_conversion_details(
                if en {
                    "Skipped: Steam Library (on another drive E:\\)"
                } else {
                    "跳过：Steam 游戏库（位于不同驱动器 E:\\）"
                }
                .into(),
            );
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-empty" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_items(ModelRc::default());
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-rename" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_inline_kind(1);
            ui.set_inline_text(if en { "Daily Apps" } else { "日常应用" }.into());
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "main-delete" => {
            let ui = MainWindow::new().map_err(|e| e.to_string())?;
            configure_main(&ui, data, false, en);
            ui.set_collection_delete(true);
            ui.set_delete_is_group(false);
            ui.set_delete_name(
                if en {
                    "Common / Temp Tools"
                } else {
                    "常用 / 临时工具"
                }
                .into(),
            );
            ui.set_delete_has_content(true);
            ui.set_delete_discard(false);
            ui.set_delete_names(if en {
                strings(&[
                    "Common / Daily Apps".into(),
                    "Common / Dev Tools".into(),
                    "Dev / Editors & IDEs".into(),
                ])
            } else {
                strings(&[
                    "常用 / 日常应用".into(),
                    "常用 / 开发工具".into(),
                    "开发 / 编辑器与 IDE".into(),
                ])
            });
            ui.set_delete_target(0);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "editor-program" => {
            let ui = EditorWindow::new().map_err(|e| e.to_string())?;
            ui.global::<I18n>().set_en(en);
            ui.set_window_active(true);
            ui.set_editing(true);
            ui.set_kind(0);
            ui.set_entry_name("VS Code".into());
            ui.set_entry_icon(data.editor_program_icon.clone());
            ui.set_collection_id(data.active_category);
            ui.set_collection_name(
                if en {
                    "Common / Daily Apps"
                } else {
                    "常用 / 日常应用"
                }
                .into(),
            );
            ui.set_target(r"C:\Program Files\Microsoft VS Code\Code.exe".into());
            ui.set_arguments("--reuse-window".into());
            ui.set_working_dir(r"D:\Workspace\Projects".into());
            ui.set_elevated(false);
            ui.set_show_mode(0);
            ui.set_aliases(
                if en {
                    "vscode, code, editor"
                } else {
                    "vscode, code, 编辑器"
                }
                .into(),
            );
            ui.set_notes(
                if en {
                    "Primary code editor with launch flags and custom working directory."
                } else {
                    "主力代码编辑器，支持命令行参数与独立工作目录。"
                }
                .into(),
            );
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "editor-url" => {
            let ui = EditorWindow::new().map_err(|e| e.to_string())?;
            ui.global::<I18n>().set_en(en);
            ui.set_window_active(true);
            ui.set_editing(false);
            ui.set_kind(3);
            ui.set_entry_name("GitHub".into());
            ui.set_entry_icon(data.editor_url_icon.clone());
            ui.set_collection_id(data.active_category);
            ui.set_collection_name(
                if en {
                    "Common / Web Docs"
                } else {
                    "常用 / 在线文档"
                }
                .into(),
            );
            ui.set_target("https://github.com".into());
            ui.set_aliases(
                if en {
                    "gh, git, repos"
                } else {
                    "gh, git, 代码托管"
                }
                .into(),
            );
            ui.set_notes(
                if en {
                    "Open-source repositories and issue tracking."
                } else {
                    "开源代码仓库与问题跟踪主页。"
                }
                .into(),
            );
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "editor-app-picker" => {
            let ui = EditorWindow::new().map_err(|e| e.to_string())?;
            ui.global::<I18n>().set_en(en);
            ui.set_window_active(true);
            ui.set_editing(false);
            ui.set_kind(5);
            ui.set_collection_id(data.active_category);
            ui.set_collection_name(
                if en {
                    "Common / Daily Apps"
                } else {
                    "常用 / 日常应用"
                }
                .into(),
            );
            ui.set_picker_open(true);
            ui.set_picker_shown(model(data.picker_apps.clone()));
            if let Some(first) = data.picker_apps.first() {
                ui.set_picker_aumid(first.aumid.clone());
                ui.set_picker_name(first.name.clone());
            }
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "editor-collection-tree" => {
            let ui = EditorWindow::new().map_err(|e| e.to_string())?;
            ui.global::<I18n>().set_en(en);
            ui.set_window_active(true);
            ui.set_editing(true);
            ui.set_kind(0);
            ui.set_entry_name("VS Code".into());
            ui.set_entry_icon(data.editor_program_icon.clone());
            ui.set_collection_id(data.active_category);
            ui.set_collection_name(
                if en {
                    "Common / Daily Apps"
                } else {
                    "常用 / 日常应用"
                }
                .into(),
            );
            ui.set_collection_open(true);
            ui.set_collection_nodes(model(data.batch_nodes.clone()));
            ui.set_collection_cursor(1);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-general" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(0);
            ui.set_hover_switch(true);
            ui.set_hover_delay(350);
            ui.set_type_to_search(true);
            ui.set_hide_after_launch(false);
            ui.set_double_click(false);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-summon" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(1);
            ui.set_hotkey_text(raptim::hotkey::DEFAULT.into());
            ui.set_edge_hide(true);
            ui.set_edge_strip_visible(true);
            ui.set_edge_strip_width(3);
            ui.set_edge_collapse_delay(250);
            ui.set_keep_topmost(false);
            ui.set_hide_on_focus_loss(false);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-protection" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(2);
            ui.set_fullscreen_protection(true);
            ui.set_protection_bypass(false);
            ui.set_protection_log(false);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-data" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(3);
            ui.set_relative_paths(true);
            ui.set_has_error_log(true);
            ui.set_restore_confirm(false);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-about" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(4);
            ui.set_legal_open(false);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        "settings-notices" => {
            let ui = SettingsWindow::new().map_err(|e| e.to_string())?;
            configure_settings(&ui, data, dark, en);
            ui.set_page(4);
            ui.set_legal_open(true);
            ui.set_legal_tab(1);
            platform.render_component(&ui, dark, scale, scene.width, scene.height)
        }
        other => Err(format!("未知预览场景：{other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn scene_ids_are_unique_kebab_case_and_sized() {
        let mut seen = HashSet::new();
        for s in SCENES {
            assert!(!s.id.is_empty());
            assert!(
                s.id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "invalid scene id: {}",
                s.id
            );
            assert!(seen.insert(s.id), "duplicate scene id: {}", s.id);
            assert!(
                !s.group_en.is_empty() && !s.title_en.is_empty() && !s.description_en.is_empty()
            );
            assert!(s.width > 0.0 && s.height > 0.0);
        }
    }

    #[test]
    fn showcase_preset_covers_every_window_group() {
        let groups: HashSet<&str> = SCENES
            .iter()
            .filter(|s| s.in_showcase)
            .map(|s| s.group)
            .collect();
        assert!(groups.contains("主窗口"));
        assert!(groups.contains("编辑窗口"));
        assert!(groups.contains("设置窗口"));

        let groups_en: HashSet<&str> = SCENES
            .iter()
            .filter(|s| s.in_showcase)
            .map(|s| s.group_en)
            .collect();
        assert!(groups_en.contains("Main Window"));
        assert!(groups_en.contains("Editor"));
        assert!(groups_en.contains("Settings"));
    }
}
