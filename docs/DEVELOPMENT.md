← 返回 [README](../README.md)

# 开发与构建

本文档说明 Raptim 的开发环境、编译配置、验证脚本、测试边界与版本/更新日志维护流程。

- [环境要求](#环境要求)
- [构建与运行](#构建与运行)
- [质量检查与验证门禁](#质量检查与验证门禁)
- [测试与回归边界](#测试与回归边界)
- [提交与协作规范](#提交与协作规范)
- [版本与更新日志流程](#版本与更新日志流程)
- [第三方组件与许可维护](#第三方组件与许可维护)

## 环境要求

| 组件 | 要求 | 说明 |
| --- | --- | --- |
| 操作系统 | Windows 10 / 11 (x64) | 仅支持 Windows 目标平台，Manifest 启用 PerMonitorV2 DPI |
| Rust 工具链 | Rust 1.92+（Edition 2024） | `Cargo.toml` 声明 `rust-version = "1.92"`；工程曾于 Rust 1.98.1 验证 |
| C++ 工具链 | Visual Studio C++ Build Tools + Windows SDK | MSVC 链接以及编译 `raptim.rc` / `libsqlite3-sys` 所需，需包含 `rc.exe` |
| 依赖锁 | `Cargo.lock`（使用 `--locked`） | Slint 系列 crate 严格锁定 `=1.18.0` |

## 构建与运行

```powershell
git clone https://github.com/KHWLGH/Raptim.git
cd Raptim

# 运行 release 版本（使用默认 EXE 同级目录数据）
cargo run --release --locked

# 隔离数据目录运行，避免污染日常工作数据
cargo run --release --locked -- --data-dir C:\Temp\RaptimTrial
```

仅编译可发布二进制：

```powershell
cargo build --release --locked
```

产物位于 `target/release/raptim.exe`（发布配置启用 `opt-level = "z"`、`lto = "thin"` 与 `strip = true`）。

### 构建配置说明

- **`.cargo/config.toml`（必须保留）**：通过 `[env] CFLAGS` 为 MSVC 传入 `/USQLITE_ENABLE_FTS5 /USQLITE_ENABLE_FTS3 ... /DSQLITE_OMIT_JSON /DSQLITE_OMIT_LOAD_EXTENSION`，裁剪 SQLite 未使用的全文检索、RTree、JSON1 与动态扩展模块，显著减小二进制体积；删除或遗漏该文件会静默改变出厂产物。
- **`build.rs`**：编译期自动探测 Windows SDK `rc.exe` 编译 `raptim.rc`（嵌入 `ui/icons/raptim.ico`）、通过链接参数嵌入 `raptim.manifest`，抓取本机 `rustc --version` 写入环境变量 `RAPTIM_RUST_VERSION` 供关于页展示，并以 `fluent` 样式编译 `ui/app.slint`。

## 质量检查与验证门禁

仓库提供全程无头（无需前台窗口）的一键验证脚本 `scripts/verify.ps1`（文件保持纯 ASCII，确保兼容 Windows PowerShell 5.1）：

```powershell
# 默认门禁：fmt 检查 + clippy(-D warnings) + debug 单元测试 + release 构建 + exe 体积输出
powershell -File scripts/verify.ps1

# 同一组门禁，clippy 与单元测试也走 release profile
powershell -File scripts/verify.ps1 -Release
```

脚本按顺序执行以下四项检查，任一环节失败即中止：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --locked -- -D warnings`
3. `cargo test --all-targets --locked`
4. `cargo build --release --locked`（完成后打印 `raptim.exe` 大小与构建时间）

### 定向单元测试与本地调试

日常仅改动单个业务模块时，可先运行定向测试加速反馈，交付前再运行 `scripts/verify.ps1`：

```powershell
# 仅运行库测试或指定模块测试
cargo test --lib --locked
cargo test --lib --locked storage::
cargo test --lib --locked menus::

# 仅运行二进制入口内的单元测试
cargo test --bin raptim --locked
```

### Windows 实时防护拦截处理

若构建或运行 `scripts/verify.ps1` 期间遇到 `os error 32` / `Access is denied`（常见于杀毒软件实时扫描 `build-script-build.exe`），脚本会自动捕获并给出提示。可通过以下任一方式处理：

- 将仓库 `target` 目录、`~/.cargo` 与 `~/.rustup` 加入安全软件排除列表；或
- 使用独立目标目录与单并发参数重试：

```powershell
powershell -File scripts/verify.ps1 -TargetDir C:\Temp\raptim-target -Jobs 1
```

## 测试与回归边界

当前自动化测试聚焦于**核心纯逻辑与关键数据不变量**，不在 CI/常规提交中维护脆弱的界面坐标、像素色值或静态文案断言：

- **自动化单元测试覆盖范围**：
  - `model` / `paths` / `storage`：目标校验、网址补全、相对路径往返、`dup_key` 统一查重、已含撞车库兼容打开、JSON v2 导出/恢复事务与设置校验。
  - `search`：排除文件路径、全拼/首字母/驼峰/多词匹配与增量更新。
  - `docking` / `windowing` / `policy` / `session`：停靠过伸几何、阻尼动画与失焦淡出曲线、物理更新矩形转换、保护决策优先级、轮询间隔与 `NoticeQueue` 合并规则。
  - `menus`：五类菜单树命令号唯一性及与 `region()` 双向路由一致性。
  - `gestures` / `diagnostics` / `platform` / `main`：180 ms/12 DIP 拖拽防误触、IME 组合保护、日志脱敏与按行半段截断、`.lnk`/`.url` 读写、网页图标标签解析与状态校验、`AppOption`/等待结果分类、字符搜索按键过滤等。
- **需人工真机验收的边界**：
  - 真实浏览器（Edge / Chrome / Firefox 等）OLE 拖入交互与各种 URL 剪贴板格式。
  - 多显示器热插拔、混合 DPI 跨屏移动以及贴边滑出时的合成器视觉观感。
  - Windows Narrator（讲述人）与系统高对比度主题兼容性。
  - 各类全屏游戏（如 `osu!lazer`、独占全屏与无边框全屏）下游戏勿扰保护的实机触发与解除恢复。

## 提交与协作规范

- **提交前缀**：沿用 `feat:`、`fix:`、`docs:`、`perf:`、`refactor:`、`test:`、`chore:` 简洁前缀描述改动意图。
- **交付前自检**：
  - 核对 `git diff` 与 `git status`，确保未误提交测试数据或 `target/` 之外的临时产物（`.gitignore` 仅忽略 `/target`）。
  - 行为、设置项、快捷键、存储结构或验证方式发生变化时，同步更新 `docs/` 对应文档与 `changelog/unreleased/zh-CN.md`。
  - 遵循「先改原因或边界再交付」，交付报告中明确区分已运行的自动化检查与尚未真机实测的路径。

## 版本与更新日志流程

仓库遵循「单索引 + 分版本正文」的更新日志组织方式：

- **日常开发**：只更新 [changelog/unreleased/zh-CN.md](../changelog/unreleased/zh-CN.md)，按 `新增`、`变更`、`修复`、`性能与维护` 四个小节记录用户可感知的变更及重大技术边界调整；正文中的仓库内链接以文件所在目录为基准（例如 `../../docs/USAGE.md`）。
- **准备正式版本**：
  1. 整理校对 `changelog/unreleased/zh-CN.md`，将其归档至 `changelog/<version>/zh-CN.md`（首行标题使用 `# <version> - YYYY-MM-DD`）。
  2. 在根目录 [CHANGELOG.md](../CHANGELOG.md) 的 `<!-- releases:start -->` 下方插入新版本入口并链接到归档文件，同时保留 `[Unreleased]` 入口并将 `changelog/unreleased/zh-CN.md` 重置为 `# Unreleased`。
  3. 同步更新版本号：同步修改 [Cargo.toml](../Cargo.toml) 的 `version` 与 [Cargo.lock](../Cargo.lock) 中的包版本（「设置 → 关于」与环境信息复制已自动读取 `CARGO_PKG_VERSION`，无需再手工改动 `ui/app.slint`）。
  4. 运行 `powershell -File scripts/verify.ps1 -Release` 验证全套门禁通过并复核相关文档链接后，再提交发布或打 Tag。

> 普通功能开发与文档整理不自动升版或归档日志，仅在明确准备发版时执行上述归档步骤。

## 第三方组件与许可维护

- 本项目自有代码采用 **GNU General Public License v3.0 only（`GPL-3.0-only`）**，Slint 1.18.0 同步选择 `GPL-3.0-only` 许可路径（并在「设置 → 关于」保留 `AboutSlint` 组件作为技术归属展示），根目录 [LICENSE](../LICENSE) 保留完整 GPLv3 原文。
- 新增或升级 Cargo 依赖时，需使用 `cargo metadata --locked --offline --filter-platform x86_64-pc-windows-msvc` 核对传递依赖许可证与 `GPL-3.0-only` 的兼容性，并同步更新 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。
- 公开分发编译好的二进制产物时，应按 GPLv3 第 6 条随附 [LICENSE](../LICENSE)、[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) 及对应版本完整源码（含锁定 `Cargo.lock`）。

