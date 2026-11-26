<div align="center">

<img src="ui/icons/raptim.svg" width="96" alt="Raptim" />

# Raptim

**一款面向 Windows 的轻量本地启动器**

[![License](https://img.shields.io/badge/license-GPL--3.0--only-blue?style=flat-square)](LICENSE)
[![Stars](https://img.shields.io/github/stars/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/stargazers)
[![Issues](https://img.shields.io/github/issues/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/issues)
[![Last commit](https://img.shields.io/github/last-commit/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/commits/main)
[![Repo size](https://img.shields.io/github/repo-size/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim)
[![Top language](https://img.shields.io/github/languages/top/KHWLGH/Raptim?style=flat-square&color=CE422B)](https://github.com/KHWLGH/Raptim)

[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011%20x64-0078D4?style=flat-square&logo=windows11&logoColor=white)](#快速开始)
[![Rust](https://img.shields.io/badge/Rust-1.92%2B%20%7C%20Edition%202024-CE422B?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Slint](https://img.shields.io/badge/UI-Slint%201.18.0%20Fluent-2379F4?style=flat-square)](https://slint.dev/)
[![SQLite](https://img.shields.io/badge/storage-SQLite%20WAL-003B57?style=flat-square&logo=sqlite&logoColor=white)](docs/ARCHITECTURE.md#数据存储与兼容边界)
[![Native](https://img.shields.io/badge/native-Win32%20%7C%20No%20WebView-4B5563?style=flat-square)](docs/ARCHITECTURE.md)
[![Portable](https://img.shields.io/badge/mode-Portable%20%7C%20Offline--first-10B981?style=flat-square)](docs/USAGE.md#开始使用与启动参数)

</div>

## 项目简介

Raptim 用于集中收纳常用的程序、文件、文件夹、网址和系统应用，平时可贴边自动隐藏，需要时通过全局热键或鼠标贴边即可呼出，输入名称或拼音就能打开目标。

软件基于 **Rust**、**Slint** 与内置 **SQLite** 原生编写，不依赖 WebView、GC 运行时或后台全盘扫描，所有数据离线保存在本地目录中（仅在添加或刷新网址项目时按需请求站点图标）。

> 当前处于本地开发预览阶段（`0.0.0`），界面与文档均以简体中文维护。

## 技术栈与选型原因

| 技术层 | 选型 | 选型原因 |
| --- | --- | --- |
| 核心语言与构建 | Rust 1.92+（Edition 2024） | 原生编译且无垃圾回收停顿，不依赖额外运行时；Release 开启 `opt-level = "z"`、`lto = "thin"` 与 `strip = true`，直接产出轻巧的单可执行文件 |
| 界面与渲染 | Slint 1.18.0（Fluent 样式、winit 后端与软件渲染器） | `.slint` 声明式界面在编译期转为机器码，避开 WebView / Electron 的浏览器进程开销；选用 CPU 软件渲染器替代 Qt 与 GPU 后端，避免常驻显存和着色器缓存，更适合长时间驻留后台的轻量工具 |
| 本地存储 | `rusqlite`（内嵌 SQLite，WAL 模式） | 零外部数据库服务，单文件即可承载配置、分组与图标；编译期通过 `.cargo/config.toml` 裁剪 FTS3/FTS5、RTree、JSON1 等未使用模块，减少体积与冗余开销 |
| Windows 系统集成 | `windows` 0.62 与 `tray-icon` | 直接调用 Win32 Shell、DWM、OLE 拖放、全局热键、全屏通知判定与 WinHTTP，避免重型中间层，能精细控制贴边、置顶和游戏勿扰等桌面行为 |
| 精简依赖策略 | `image`、`pinyin` 等 | 严格按需开启特性（如 `image` 仅保留 PNG/JPEG/ICO，`pinyin` 仅保留无声调 `plain` 词表），配合工作线程异步处理耗时任务，减少二进制体积与常驻内存 |

更多架构细节与实测数据见[技术架构](docs/ARCHITECTURE.md)和[性能基准与边界](docs/PERFORMANCE.md)。

## 核心功能

- **两级分组收纳**：支持一级分组与二级分类，可统一管理程序、文件、文件夹、网址、快捷方式和 Microsoft Store / UWP 应用，并支持拖拽排序、跨组整理以及小/中/大图标网格与列表视图切换。
- **拼音与键盘快搜**：按名称、别名和备注即时筛选，支持中文全拼、首字母与多关键词匹配，也可开启获焦后直接打字搜索；呼出、检索、切换分类、启动或以管理员身份运行都可全程用键盘完成。
- **快捷收录与批量维护**：支持直接拖入本地文件和浏览器网址，提供开始菜单与已安装应用扫描导入，可多选批量移动、复制、移除项目，并支持检查失效条目、就地重定位以及相对/绝对路径批量转换。
- **贴边收纳与快捷唤出**：窗口可停靠在屏幕左、右或顶部边缘自动收起，鼠标贴边、全局热键（默认 `Ctrl+Alt+Space`）或点击托盘图标均可随时唤出；支持自定义露边宽度、收起延迟、失焦隐藏、窗口置顶和开机静默启动。
- **游戏全屏勿扰**：自动识别全屏游戏或演示场景，并支持手动添加严格保护程序，避免游戏过程中误触热键或鼠标贴边弹出窗口；需要时也可在托盘中临时暂停。
- **离线本地与数据迁移**：所有配置、分组和图标默认存放在程序同级目录的 `raptim.db` 中，支持 JSON 导出与导入恢复，也能通过 `--data-dir` 指定独立数据目录，方便多套配置隔离或整目录迁移。

## 快速开始

### 环境要求

- Windows 10 / 11 (x64)
- Rust 1.92+（Edition 2024）
- Visual Studio C++ Build Tools 与 Windows SDK（含 `rc.exe`）

### 构建与运行

```powershell
cargo run --release --locked
```

编译后的可执行文件位于 `target/release/raptim.exe`。支持以下启动模式：

```powershell
# 便携模式（与默认行为一致，直接读写 EXE 同级目录）
.\target\release\raptim.exe --portable

# 独立数据目录（适合开发、测试与多库隔离）
.\target\release\raptim.exe --data-dir C:\Temp\RaptimTrial

# 静默启动（已贴边时直接收起，否则隐藏到系统托盘）
.\target\release\raptim.exe --silent
```

> **备份提示**：软件不提供内置自动备份。如需备份或替换数据，请先退出 Raptim，再将整个数据目录（包含 `raptim.db`、`raptim.db-wal` 与 `raptim.db-shm`）一并复制，详见[使用指南](docs/USAGE.md#数据备份导出恢复与故障排查)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [使用指南](docs/USAGE.md) | 启动参数、日常操作、快捷键、全部设置项与数据恢复说明 |
| [技术架构](docs/ARCHITECTURE.md) | 模块划分、事件模型、SQLite v2 表结构、防打扰状态机与窗口边界 |
| [开发与构建](docs/DEVELOPMENT.md) | 环境要求、编译配置、无头验证脚本、测试边界与更新日志流程 |
| [性能基准与边界](docs/PERFORMANCE.md) | 存储与启动实测数据、图标缓存策略及 DWM 合成器已知边界 |
| [更新日志](CHANGELOG.md) | 版本发布索引与当前未发布改动记录（[Unreleased 草稿](changelog/unreleased/zh-CN.md)） |
| [许可证 (GPL-3.0-only)](LICENSE) · [第三方组件](THIRD_PARTY_NOTICES.md) | GNU GPLv3 协议全文、直接依赖许可核查与内置资产归属 |

## 开发验证

仓库提供无需弹出窗口的本地验证脚本，可一键执行格式化检查、Clippy、单元测试与 Release 构建：

```powershell
powershell -File scripts/verify.ps1           # 运行 fmt、clippy(-D warnings)、单元测试与 release 构建
powershell -File scripts/verify.ps1 -Release  # 在 release 模式下执行完整验证
```

如需自定义构建目录、限制并发数或了解完整参数，见[开发与构建指南](docs/DEVELOPMENT.md#质量检查与验证门禁)。

## 许可证

Copyright (C) 2026 KHWLGH

本软件基于 **GNU General Public License v3.0（`GPL-3.0-only`）** 开源发布，完整协议见 [LICENSE](LICENSE)，按现状提供且不附带任何担保。

Slint 1.18.0 同步采用 `GPL-3.0-only` 许可路径（「设置 → 关于」界面保留官方 `AboutSlint` 组件作为归属展示），其余传递与直接依赖均使用与 GPL-3.0-only 兼容的宽松开源许可。内置 `ui/icons/rust.svg` 遵循 CC BY 4.0 协议，完整依赖与资产清单见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 相关链接

- [提交 Issue 或建议](https://github.com/KHWLGH/Raptim/issues)
- [Slint 官方网站](https://slint.dev/)
- [Rust 官方网站](https://www.rust-lang.org/)
