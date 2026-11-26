<div align="center">

<img src="ui/icons/raptim.svg" width="96" alt="Raptim" />

# Raptim

**轻量级 Windows 本地快速启动器 —— 快速检索 · 贴边收纳 · 游戏勿扰 · 单目录便携**

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

Raptim 是一款面向 Windows 的本地快速启动器，基于 **Rust** 与 **Slint 1.18.0（Fluent 控件 · winit 后端 · 软件渲染器）** 构建，使用内置 **SQLite（WAL 模式）** 在单个数据目录保存全部配置、分组与图标。应用不依赖 WebView、JavaScript 或 C#，不内置磁盘索引（Everything）、账号、云服务或换肤系统，仅在用户保存/拖入网址或手动刷新网页图标时按需请求目标站点图标。

> **当前状态**：本地开发预览版本（`0.0.0`），界面文案与文档以简体中文维护。

## 功能特性

### 组织与视图

- **双层分类结构**：标题栏下方提供可横向滚动的一级分组栏，左侧提供支持名称换行的纵向子分类栏；两栏尾部均配备就近「＋」按钮新建分组/子分类（空白处不弹菜单），支持 `F2` 就地改名、同级拖拽排序、分类跨分组移动及迁移/直接删除约束。
- **自适应多视图**：项目区支持小（最多 5 列）/ 中（最多 4 列）/ 大（最多 3 列）图标网格与单列列表视图，窄窗自动减少列数；悬停显示名称、完整路径或网址及最多 6 行备注，状态提示条 6 秒自动隐藏并合并重复提示。
- **单页滚动编辑**：支持程序、文件、文件夹、网址（自动补全 `https://`）、快捷方式、打包应用（`shell:AppsFolder` 列表选择）六类目标；程序与快捷方式支持普通 / 最大化 / 最小化打开方式，编辑窗支持未保存保护与拖入单张图片作图标。

### 快速检索与键盘流

- **窗内搜索浮层**：支持「全部 / 当前分组 / 当前分类」三种范围，对名称、别名、备注（不匹配路径）进行多关键词、全拼及逐字/词首/驼峰首字母相关度检索；内置 IME 输入法组合态保护，并可选开启「获焦后直接打字搜索」。
- **完整键盘操作**：支持 `Ctrl+F` 搜索、`Ctrl+N` 新建、`Ctrl+,` 打开设置、`Ctrl+Tab`/`Ctrl+Shift+Tab` 切换分类、方向键/`Home`/`End` 移动焦点、`Enter` 启动、`Shift+Enter` 管理员打开、`Alt+Enter`/`F2` 编辑或改名、`Delete` 删除以及 `Esc` 分层关闭。

### 多途径导入与批量管理

- **外部拖入与软件扫描**：接入原生 OLE 文件与浏览器网址拖入（拖至分类节点直接归入该分类），提供「扫描导入软件」（检索开始菜单快捷方式与已安装应用）和「检查失效项目」（后台巡检并提供重定位/编辑/移除）。
- **多选整理与路径转换**：支持 `Shift` 连选、`Space` 勾选、`Ctrl+A` 全选批量移动/复制/删除（保留原文件，遇重复目标取消整批）；支持按分组/分类树形多选并将程序路径与工作目录批量转换为相对或绝对路径。

### 贴边收纳与游戏勿扰

- **阻尼贴边收纳**：支持左/右/上三边停靠与临界阻尼动画，露边宽度支持 **1–12 物理像素**（可隐藏露边并用悬停或热键唤出），支持 150 / 250 / 450 ms 收起延迟、贴边过伸隐藏贴边侧系统圆角与「窗口始终置顶」。
- **多渠道唤出与失焦隐藏**：关闭按钮语义为隐藏窗口，支持自定义全局热键（默认 `Ctrl+Alt+Space`）、系统托盘（左键唤出，右键含本次有效的「暂停自动隐藏」与「暂停游戏勿扰」）、按数据目录隔离的第二实例唤醒及开机启动（`--silent` 静默启动）。
- **全屏与严格程序保护**：结合全屏窗口几何、`SHQueryUserNotificationState` 与最多 100 项「严格保护程序」拦截游戏期间误呼出；支持按需记录脱敏保护判定日志（`protection-log.csv`）。

### 本地便携与数据边界

- **单目录存储**：默认在 `raptim.exe` 同级目录保存 `raptim.db`（格式版本 2，独立 `icon BLOB` 列与物化 `dup_key` 去重键），支持 JSON 版本 2 导出与事务原子恢复；常开脱敏故障日志 `error-log.txt`。
- **路径不可达就地重定位**：区分启动失败原因，仅对目标缺失、设备未就绪、网络不可用及无效路径四类有文件路径的目标提供就地重定位（保留参数、工作目录与打开方式）。

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

# 静默启动（停靠时以无焦点露边收纳，否则隐藏驻留托盘）
.\target\release\raptim.exe --silent
```

> **数据提示**：Raptim 只读写格式版本 2 的数据库与 JSON 导出，不提供内置自动备份。正式替换或备份数据库前，请先退出程序并将整个数据目录（含 `raptim.db`、`-wal`、`-shm`）一并复制；详见[使用指南](docs/USAGE.md#数据备份导出恢复与故障排查)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [使用指南](docs/USAGE.md) | 启动参数、界面操作、快捷键表、设置项说明、拖入/扫描/重定位与数据恢复 |
| [技术架构](docs/ARCHITECTURE.md) | 模块职责、事件模型、SQLite v2 表结构与 `dup_key` 语义、显示保护状态机与渲染桥接 |
| [开发与构建](docs/DEVELOPMENT.md) | 环境准备、`.cargo/config.toml` 说明、无头验证门禁、测试边界与更新日志流程 |
| [性能基准与边界](docs/PERFORMANCE.md) | 存储 v2 实测对比、图标虚拟化与 24 MiB 缓存策略、DWM 合成器边界与否决项记录 |
| [更新日志](CHANGELOG.md) | 版本索引与当前未发布改动记录（[Unreleased 草稿](changelog/unreleased/zh-CN.md)） |
| [许可证 (GPL-3.0-only)](LICENSE) · [第三方组件](THIRD_PARTY_NOTICES.md) | GNU GPLv3 原文与全量直接依赖、Rust 标志及内置资产归属说明 |

## 开发验证

仓库提供全程无需前台窗口的无头质量门禁脚本：

```powershell
powershell -File scripts/verify.ps1           # fmt 检查 + clippy(-D warnings) + 单元测试 + release 构建
powershell -File scripts/verify.ps1 -Release  # 同一组门禁，clippy 与单元测试也走 release profile
```

若本机杀毒软件锁定构建脚本文件或需要更多构建参数，详见[开发与构建指南](docs/DEVELOPMENT.md#质量检查与验证门禁)。

## 许可证

Copyright (C) 2026 KHWLGH

本项目自有贡献以 **GNU General Public License v3.0 only（`GPL-3.0-only`）** 开源发布，完整许可正文见 [LICENSE](LICENSE)，软件按原样提供、不作任何担保。

Slint 1.18.0 同步采用 `GPL-3.0-only` 许可路径（「设置 → 关于」保留官方 `AboutSlint` 组件作为技术归属展示）；其余全部直接与传递依赖均使用宽松开源许可并与 `GPL-3.0-only` 兼容。内置 `ui/icons/rust.svg` 遵循 CC BY 4.0，全部依赖与资产归属明细见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 相关链接

- [提交 Issue 或建议](https://github.com/KHWLGH/Raptim/issues)
- [Slint 官方网站](https://slint.dev/)
- [Rust 官方网站](https://www.rust-lang.org/)
