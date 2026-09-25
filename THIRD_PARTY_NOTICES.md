# 第三方组件说明

当前为本地开发预览，尚未完成公开分发的完整许可清单。

- Slint 1.18.0：桌面应用采用 Royalty-free Desktop, Mobile, and Web Applications License 2.0 路径；已核对锁定 crate 中的 `LICENSES/LicenseRef-Slint-Royalty-free-2.0.md`。“设置 → 关于”展示官方 `AboutSlint` 组件。[许可原文](https://slint.dev/license/royalty-free)。
- pinyin 0.10：MIT；用于离线拼音/首字母检索。
- arboard 3.6：MIT / Apache-2.0；用于复制项目路径或网址。
- windows-rs：MIT / Apache-2.0。
- rusqlite：MIT；内置 SQLite 按上游公共领域声明提供。
- image、winit、tray-icon、rfd、serde、serde_json、url 等及其传递依赖，以锁定版本的上游许可为准，正式分发前生成完整清单和随附声明。

`ui/icons/rust.svg`：Rust 项目官方标志，来源 https://github.com/rust-lang/rust-artwork/blob/main/logo/rust-logo.svg ，CC BY 4.0（https://creativecommons.org/licenses/by/4.0/）。原文件未修改；界面随主题调整显示颜色。Rust 名称与标志属于其权利人，此处仅用于技术归属说明。

其余 `ui/icons/` 通用 SVG 与托盘花形标记为本项目新建资产，未使用研究概念图中的第三方应用图标。未导入字体，界面请求本机系统字体。

Windows Shell 图标从用户本机目标提取并保存于用户数据库；不会作为项目内置第三方应用图标发布。
