# 第三方组件说明

本项目自有贡献采用 **GNU General Public License v3.0 only（`GPL-3.0-only`）** 发布，`Copyright (C) 2026 KHWLGH`，完整许可正文见 [LICENSE](LICENSE)。本文件记录 Raptim 所使用的第三方依赖、许可兼容情况以及内置图形资源的归属声明。

## Slint 与直接依赖清单

以下版本与许可证信息基于当前锁定配置 `Cargo.lock`（目标平台 `x86_64-pc-windows-msvc`）核实：

| 组件 | 锁定版本 | 上游许可证 | 用途说明 |
| --- | --- | --- | --- |
| `slint` / `i-slint-core` / `slint-build` | `1.18.0` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | **本项目采用 `GPL-3.0-only` 路径**；「设置 → 关于」保留官方 `AboutSlint` 组件作为技术归属展示 |
| `rusqlite` | `0.37.0` | `MIT` | 本地数据持久化；`bundled` 特性内嵌 SQLite 源码（上游公共领域声明） |
| `windows` / `windows-core` | `0.62.2` | `MIT OR Apache-2.0` | Win32 窗口、Shell、DWM、WinHTTP、OLE 拖放与热键接口 |
| `tray-icon` | `0.24.2` | `MIT OR Apache-2.0` | Windows 系统托盘图标与原生托盘菜单 |
| `rfd` | `0.16.0` | `MIT` | 系统原生文件/目录选择框与错误提示对话框 |
| `image` | `0.25.10` | `MIT OR Apache-2.0` | PNG / JPEG / ICO 图标读取、缩放与编码 |
| `pinyin` | `0.10.0` | `MIT` | 离线无声调拼音与首字母检索（`plain` 特性） |
| `arboard` | `3.6.1` | `MIT OR Apache-2.0` | 复制项目路径或网址到系统剪贴板 |
| `base64` | `0.23.1` | `MIT OR Apache-2.0` | JSON 导出中的图标字节编码与解码 |
| `serde` / `serde_json` | `1.0.229` / `1.0.151` | `MIT OR Apache-2.0` | 设置、本机状态与 JSON v2 导出序列化 |
| `url` | `2.5.8` | `MIT OR Apache-2.0` | 网址格式解析、规范化与图标链接拼接 |
| `thiserror` | `2.0.20` | `MIT OR Apache-2.0` | 统一错误类型派生 |
| `raw-window-handle` | `0.6.2` | `MIT OR Apache-2.0 OR Zlib` | 跨层读取 Win32 原生 `HWND` 句柄 |
| `tempfile` | `3.27.0` | `MIT OR Apache-2.0` | 仅 `dev-dependencies`，用于单元测试隔离临时目录，不进入发行二进制 |

经离线 `cargo metadata --locked --filter-platform x86_64-pc-windows-msvc` 核对，构建图中全部非 Slint 传递依赖均采用宽松许可（`MIT`、`Apache-2.0`、`BSD-2-Clause`、`BSD-3-Clause`、`ISC`、`Zlib`、`Unlicense`、`CC0-1.0`、`Unicode-3.0`、`BSL-1.0`、`0BSD` 及其多选组合），均与 `GPL-3.0-only` 兼容。公开分发二进制产物时，应随附 [LICENSE](LICENSE) 并按 GPLv3 第 6 条提供对应版本源码（含锁定 `Cargo.lock`）。

## 图形资产与本机提取图标

- `ui/icons/rust.svg`：Rust 项目官方标志，来源 `https://github.com/rust-lang/rust-artwork/blob/main/logo/rust-logo.svg`，采用 **CC BY 4.0**（`https://creativecommons.org/licenses/by/4.0/`）。原文件未修改，界面仅随主题动态着色；Rust 名称与标志属于其权利人，此处仅用于「设置 → 关于」中的技术归属展示。
- 其余 `ui/icons/` 通用 SVG 与托盘图标 `ui/icons/raptim-tray.png` / `raptim.ico` / `raptim.svg` 均为本项目自建资产，未包含第三方专有应用图标；界面不内置字体文件，运行期请求本机系统字体。
- Windows Shell 图标、已安装应用图标及网页站点图标均于运行期从用户本机或用户指定网址提取，仅保存在用户本地 `raptim.db` 中，不会随本项目源码或安装包分发。
