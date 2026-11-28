<div align="center">

<img src="ui/icons/raptim.svg" width="96" alt="Raptim" />

# Raptim

**一款面向 Windows 的轻量本地启动器**

**简体中文** | [English](README.en.md)

[![License](https://img.shields.io/badge/license-GPL--3.0--only-blue?style=flat-square)](LICENSE)
[![Stars](https://img.shields.io/github/stars/KHWLGH/Raptim?style=flat-square)](https://github.com/KHWLGH/Raptim/stargazers)
[![Downloads](https://img.shields.io/github/downloads/KHWLGH/Raptim/total?style=flat-square)](https://github.com/KHWLGH/Raptim/releases)
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

Raptim 可以把常用的程序、文件、文件夹、网址和系统应用集中收在一起。平时停靠在屏幕边缘自动隐藏，需要时按快捷键或把鼠标移到屏边就能呼出，敲名字或拼音就能打开目标。

程序使用 **Rust**、**Slint** 和内置 **SQLite** 原生构建，不依赖 WebView 或外部运行库，也不做后台全盘扫描。所有配置、分组和图标都离线保存在本地目录里，只在添加或刷新网址时联网获取网站图标。

> 当前版本 `0.1.0`，支持 **简体中文** 与 **English** 双语界面及文档（首次启动自动匹配 Windows 系统语言，也可在「设置 → 常规 → 界面语言」中实时切换）。

## 界面预览

<p align="center">
  <img src="docs/previews/overview-light.png#gh-light-mode-only" alt="Raptim 界面总览" width="100%" />
  <img src="docs/previews/overview-dark.png#gh-dark-mode-only" alt="Raptim 界面总览" width="100%" />
</p>

<p align="center">
  <sub>主窗口网格、快速搜索、项目编辑与设置界面总览（以上截图随 GitHub 浅色 / 深色主题自动切换）</sub>
</p>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>主窗口 · 网格视图</strong><br />
      <sub>一级分组栏、二级分类导航与多列项目网格</sub><br /><br />
      <img src="docs/previews/light/main-grid.png#gh-light-mode-only" alt="主窗口网格视图" width="320" />
      <img src="docs/previews/dark/main-grid.png#gh-dark-mode-only" alt="主窗口网格视图" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>主窗口 · 快速搜索</strong><br />
      <sub>支持全部 / 分组 / 分类范围与中文全拼、首字母检索</sub><br /><br />
      <img src="docs/previews/light/main-search.png#gh-light-mode-only" alt="主窗口搜索浮层" width="320" />
      <img src="docs/previews/dark/main-search.png#gh-dark-mode-only" alt="主窗口搜索浮层" width="320" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>扫描导入已安装软件</strong><br />
      <sub>自动识别开始菜单与系统应用，支持关键字筛选批量导入</sub><br /><br />
      <img src="docs/previews/light/main-discover.png#gh-light-mode-only" alt="扫描导入已安装软件" width="320" />
      <img src="docs/previews/dark/main-discover.png#gh-dark-mode-only" alt="扫描导入已安装软件" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>项目编辑表单</strong><br />
      <sub>支持程序、文件、文件夹、网址、快捷方式与 UWP 应用</sub><br /><br />
      <img src="docs/previews/light/editor-program.png#gh-light-mode-only" alt="项目编辑窗口" width="360" />
      <img src="docs/previews/dark/editor-program.png#gh-dark-mode-only" alt="项目编辑窗口" width="360" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>设置 · 呼出与贴边收纳</strong><br />
      <sub>全局热键录制、贴边自动隐藏、露边宽度与始终置顶</sub><br /><br />
      <img src="docs/previews/light/settings-summon.png#gh-light-mode-only" alt="设置呼出与隐藏页" width="420" />
      <img src="docs/previews/dark/settings-summon.png#gh-dark-mode-only" alt="设置呼出与隐藏页" width="420" />
    </td>
    <td width="50%" align="center">
      <strong>设置 · 游戏勿扰保护</strong><br />
      <sub>全屏自动免打扰、临时旁路开关与严格保护程序列表</sub><br /><br />
      <img src="docs/previews/light/settings-protection.png#gh-light-mode-only" alt="设置游戏勿扰页" width="420" />
      <img src="docs/previews/dark/settings-protection.png#gh-dark-mode-only" alt="设置游戏勿扰页" width="420" />
    </td>
  </tr>
</table>

<details>
<summary>查看更多界面截图（列表视图、批量整理、失效检查与设置页）</summary>

<table>
  <tr>
    <td width="50%" align="center">
      <strong>主窗口 · 列表视图</strong><br />
      <sub>单列紧凑列表展示项目名称与完整目标路径</sub><br /><br />
      <img src="docs/previews/light/main-list.png#gh-light-mode-only" alt="主窗口列表视图" width="320" />
      <img src="docs/previews/dark/main-list.png#gh-dark-mode-only" alt="主窗口列表视图" width="320" />
    </td>
    <td width="50%" align="center">
      <strong>批量多选整理</strong><br />
      <sub>勾选多个条目并通过底部悬浮栏一键移动、复制或删除</sub><br /><br />
      <img src="docs/previews/light/main-batch.png#gh-light-mode-only" alt="批量多选整理" width="320" />
      <img src="docs/previews/dark/main-batch.png#gh-dark-mode-only" alt="批量多选整理" width="320" />
    </td>
  </tr>
  <tr>
    <td colspan="2" align="center">
      <strong>失效项目检查</strong><br />
      <sub>离线检测缺失文件、文件夹与已卸载应用，支持快速重定位或移除</sub><br /><br />
      <img src="docs/previews/light/main-broken.png#gh-light-mode-only" alt="失效项目检查" width="320" />
      <img src="docs/previews/dark/main-broken.png#gh-dark-mode-only" alt="失效项目检查" width="320" />
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <strong>设置 · 常规</strong><br />
      <sub>外观主题、双语界面切换、开机启动、单双击启动与悬停切换分类</sub><br /><br />
      <img src="docs/previews/light/settings-general.png#gh-light-mode-only" alt="设置常规页" width="420" />
      <img src="docs/previews/dark/settings-general.png#gh-dark-mode-only" alt="设置常规页" width="420" />
    </td>
    <td width="50%" align="center">
      <strong>设置 · 关于</strong><br />
      <sub>版本号、环境信息一键复制、GPLv3 许可与第三方组件归属</sub><br /><br />
      <img src="docs/previews/light/settings-about.png#gh-light-mode-only" alt="设置关于页" width="420" />
      <img src="docs/previews/dark/settings-about.png#gh-dark-mode-only" alt="设置关于页" width="420" />
    </td>
  </tr>
</table>

</details>

> 如需本地无头生成全部 21 个界面（含批量整理、路径转换、失效检查及图库页），可运行 `powershell -File scripts/preview.ps1 -Preset all`，详见[界面预览生成工具](docs/DEVELOPMENT.md#界面预览生成工具)。

## 技术栈与选型原因

| 技术层 | 选型 | 选型原因 |
| --- | --- | --- |
| 核心语言与构建 | Rust 1.92+（Edition 2024） | 原生编译，没有垃圾回收停顿，也不需要额外安装运行库；Release 开启 `opt-level = "z"`、`lto = "thin"` 与 `strip = true`，能直接产出体积紧凑的单文件 exe |
| 界面与渲染 | Slint 1.18.0（Fluent 样式、winit 后端与软件渲染器） | `.slint` 界面在编译期直接转为原生代码，避开了 WebView / Electron 的浏览器进程开销；渲染上选用 CPU 软件渲染器替代 Qt 与 GPU 后端，不会常驻显存和着色器缓存，更适合长期挂在后台的桌面工具 |
| 本地存储 | `rusqlite`（内嵌 SQLite，WAL 模式） | 内置单文件数据库就能存下全部配置、分组和图标，不依赖外部服务；同时通过 `.cargo/config.toml` 裁掉了 FTS、RTree、JSON1 等用不到的 SQLite 模块，进一步控制二进制体积 |
| Windows 系统集成 | `windows` 0.62 与 `tray-icon` | 直接调用 Win32 Shell、DWM、OLE 拖放、全局热键、全屏状态检测和 WinHTTP，没有多余的中间封装，便于精细控制贴边动画、置顶层级与游戏勿扰 |
| 精简依赖策略 | `image`、`pinyin` 等 | 第三方库只开启实际用到的特性（例如 `image` 仅保留 PNG/JPEG/ICO，`pinyin` 仅保留无声调词表），耗时任务放在后台线程处理，尽量减少体积和常驻内存 |

更多架构细节与实测数据见[技术架构](docs/ARCHITECTURE.md)和[性能基准与边界](docs/PERFORMANCE.md)。

## 核心功能

- **分组与视图**：支持一级分组和二级分类，可收纳程序、文件、文件夹、网址、快捷方式以及 Microsoft Store / UWP 应用，支持拖拽排序、跨组整理以及小/中/大图标网格或列表视图。
- **快速检索与键盘操作**：按名称、别名和备注即时过滤，支持中文全拼、首字母和多关键词匹配，也支持获焦后直接打字搜索；从呼出、搜索、切换分类到启动或以管理员身份运行都能全程键盘操作。
- **导入与批量整理**：支持拖入本地文件和浏览器网址，也能扫描开始菜单和已安装应用批量导入；提供多选移动、复制、移除，失效项目巡检与重定位，以及相对/绝对路径批量转换。
- **贴边收纳与随时呼出**：窗口可停靠在屏幕左、右或上边缘自动收起，鼠标贴边、全局热键（默认 `Ctrl+Alt+Space`）或托盘点击都能呼出；支持调节露边宽度、收起延迟、失焦隐藏、窗口置顶与开机静默启动。
- **游戏勿扰**：自动识别全屏游戏或演示场景，也可手动添加严格保护程序列表，游戏期间自动拦截热键和鼠标贴边弹出；需要时能在托盘菜单中临时暂停。
- **本地存储与双语界面**：配置、分组和图标全部保存在程序同级目录的 `raptim.db` 中，支持 JSON 导出与恢复，也能通过 `--data-dir` 指定独立数据目录隔离多套配置；内置中英文界面与自动系统语言匹配。

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
| [技术架构](docs/ARCHITECTURE.md) | 模块划分、事件模型、SQLite 存储结构、防打扰状态机与窗口边界 |
| [开发与构建](docs/DEVELOPMENT.md) | 环境要求、编译配置、无头验证脚本、测试边界与更新日志流程 |
| [性能基准与边界](docs/PERFORMANCE.md) | 存储与启动实测数据、图标缓存策略及 DWM 合成器已知边界 |
| [更新日志](CHANGELOG.md) | 版本发布索引与各版本变更记录（最新 [0.1.0](changelog/0.1.0/zh-CN.md) · [English](changelog/0.1.0/en-US.md)） |
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

Slint 1.18.0 同步采用 `GPL-3.0-only` 许可路径（「设置 → 关于」界面保留官方 `AboutSlint` 组件作为归属展示），其余依赖均使用与 GPL-3.0-only 兼容的宽松开源许可。内置 `ui/icons/rust.svg` 遵循 CC BY 4.0 协议，完整依赖与资产清单见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 相关链接

- [提交 Issue 或建议](https://github.com/KHWLGH/Raptim/issues)
- [Slint 官方网站](https://slint.dev/)
- [Rust 官方网站](https://www.rust-lang.org/)
