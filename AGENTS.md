# AGENTS.md

本文件为在 Raptim 仓库工作的编码助手提供项目约定与硬性约束，适用于整个仓库；用户的明确指令优先。

## 项目概览与文档入口

Raptim 是面向 Windows 的本地快速启动器，基于 **Rust（Edition 2024）**、**Slint 1.18.0（Fluent 样式 · winit 后端 · 软件渲染器）** 与内置 **SQLite（WAL 模式）** 构建。

开始修改前，请先阅读对应模块的现有实现，并按任务查阅专题文档：

- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)：环境要求、构建配置、验证门禁、测试边界与版本/更新日志维护流程。
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：模块分层、事件模型、SQLite 存储与 JSON 导出契约、策略状态机与渲染桥接。
- [docs/PERFORMANCE.md](docs/PERFORMANCE.md)：存储与图标实测数据、缓存策略、贴边改框边界与已否决的优化方向。
- [docs/USAGE.md](docs/USAGE.md)：启动参数、功能交互、快捷键、全部设置项及故障排查规则。
- [CHANGELOG.md](CHANGELOG.md) 与 [changelog/unreleased/zh-CN.md](changelog/unreleased/zh-CN.md)：版本索引与日常未发布改动草稿。

| 路径 | 职责 |
| --- | --- |
| `src/lib.rs` | 核心库入口，导出 17 个业务/平台模块并集中展开 `slint::include_modules!()` |
| `src/main.rs` · `src/app_*.rs` · `src/ui_commands.rs` | 二进制入口、`App` 状态机、编辑器/输入/设置/工具会话与命令路由适配器 |
| `src/platform/` | Windows 原生平台实现（`windows_impl.rs` 及自启动、软件发现、OLE 拖放、自绘菜单、帧时钟、热键、保护判定、Shell 操作、托盘、网页图标抓取） |
| `ui/app.slint` · `ui/chrome.slint` · `ui/icons/` | Slint 界面组件与内置矢量/托盘图标资源 |
| `scripts/verify.ps1` | 默认无头门禁脚本（`fmt` + `clippy` + `test` + `release build`） |
| `.cargo/config.toml` · `build.rs` | SQLite 编译裁剪 `CFLAGS`、manifest/图标嵌入与编译期 `RAPTIM_RUST_VERSION` 注入 |
| `docs/` · `changelog/` | 专题中文文档、版本索引与分版本更新日志 |
| `LICENSE` · `THIRD_PARTY_NOTICES.md` | GNU GPLv3（`GPL-3.0-only`）许可证原文与第三方依赖/图标资产归属清单 |

## 环境与验证命令

在仓库根目录执行 PowerShell 命令；工程依赖 Rust 1.92+、MSVC Build Tools 与 Windows SDK（含 `rc.exe`），使用 `--locked` 尊重 `Cargo.lock`：

```powershell
# 运行无头完整验证（默认 debug 测试 + release 构建）
powershell -File scripts/verify.ps1

# release profile 门禁
powershell -File scripts/verify.ps1 -Release

# 若实时防护拦截构建脚本（os error 32），使用排除目录与单并发
powershell -File scripts/verify.ps1 -TargetDir C:\Temp\raptim-target -Jobs 1
```

## 实现约定与硬性不变量

1. **版本锁定与渲染桥接**：`Cargo.toml` 中 `slint`、`i-slint-core` 与 `slint-build` 严格锁定 `=1.18.0`。`src/windowing.rs` 针对 Slint 1.18 软件渲染器实现了隐藏重显脏区标记与 `ToolTipImpl` 轮廓剥离，未经实测验证不要升级或解除版本锁定。
2. **构建配置保护**：不要删除或修改 `.cargo/config.toml` 中的 SQLite `CFLAGS`（关闭了 FTS/RTree/JSON1 等未使用模块）；`scripts/verify.ps1` 必须保持纯 ASCII，确保 Windows PowerShell 5.1 读取时不乱码。
3. **存储与去重统一口径（`dup_key`）**：
   - 数据库与导出格式当前固定校验内部 schema 标记（`user_version = 2` / `format_version = 2`）；所有改写目标的写入路径（含 `update_targets_checked`、编辑保存、拖入、批量移动/复制与恢复预检）必须共用 `storage_key` / `slots_of` / `first_clash` 判定。
   - 保持 `entries_collection(collection_id, dup_key)` 为**普通索引而非 `UNIQUE`**，以确保批处理事务中间态与已有重复行的数据库仍可正常打开；不要暗中迁移、覆盖或静默修复不兼容数据库。
4. **Windows Shell 与窗口边界**：
   - 禁止对 `SHBindToParent` 返回的 `ppidlLast`（内部子 PIDL）调用 `CoTaskMemFree`；父级 `Pidl` 必须持续存活至 `GetUIObjectOf` 完成。
   - 贴边与置顶操作禁止经由 `winit::set_window_level` 改写窗口层级（会抹除 `WS_EX_TOOLWINDOW`），且所有改框写入须先检查当前 HWND 状态。
5. **策略与网络边界**：
   - 一切显示或唤出入口必须经过 `DisplayPolicy::allows(source, ticket)` 闸门；不要绕过保护状态或伪造确认。
   - 保持本地离线优先：启动图标回填禁止联网抓取 `Target::Url` 图标；外部日志（`error-log.txt` / `protection-log.csv`）必须经 `diagnostics::redact` 对用户目录与数据目录脱敏。
6. **开源许可与依赖合规**：
   - 项目自有代码与整体分发采用 `GPL-3.0-only`（Slint 1.18.0 同步选用 GPLv3 路径，并在关于页保留 `AboutSlint` 组件作为技术归属展示）。
   - 新增或升级依赖时必须离线核查其与 `GPL-3.0-only` 的兼容性并同步更新 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 测试与回归边界

- 保持单元测试聚焦于核心纯逻辑与关键不变量（数据存储、去重、搜索、路径转换、停靠几何、保护策略、菜单树路由、日志脱敏/截断、手势/输入规则）。
- 不新增绑定具体 UI 坐标、像素色值、截图或界面文案的脆弱回归测试，不在文档整理或常规改动中编造未运行的验证结论。
- 涉及真实浏览器 OLE 拖放、混合 DPI、显示器热插拔、讲述人/高对比度及全屏游戏的表现，交付时需明确区分哪些仅通过了本地逻辑单测、哪些尚需人工真机验收。

## 文档与更新日志维护

- 仓库文档与更新日志以简体中文维护；开发期间只追加维护 `changelog/unreleased/zh-CN.md`（按 `新增`、`变更`、`修复`、`性能与维护` 分组）。
- 行为、设置项、快捷键、存储契约或构建流程发生变化时，同步更新 `docs/` 对应文档与 `README.md`；修改版本号需同步 `Cargo.toml` 与 `Cargo.lock`（关于页已自动读取 `CARGO_PKG_VERSION`）。
- 普通功能改动或文档整理**不自动升版、不归档历史日志、不自动提交或打 Git Tag**，仅在用户明确要求发布时执行发版归档流程。
