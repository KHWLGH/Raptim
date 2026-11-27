**简体中文** | [English](en-US.md)

# Unreleased

### 新增

- 新增英文界面与系统语言自适应支持（`src/i18n.rs` 与 Slint 全局 `I18n`）：首次启动自动检测 Windows UI 语言（读取注册表 `PreferredUILanguages` / `UserProfile\Languages` / `LocaleName`），新库按语言初始化默认分组/分类（「常用 / 默认分类」或 `Common / Default`），并可在「设置 → 常规 → 界面语言」中在「跟随系统 / 简体中文 / English」间实时切换（跨全部窗口、右键自绘菜单与托盘菜单即时生效，并同步切换 `Segoe UI` 与 `Microsoft YaHei UI` 字体）。
- 新增完整英文文档与 README 双语切换：[README.en.md](../../README.en.md)、[docs/en/{USAGE,ARCHITECTURE,DEVELOPMENT,PERFORMANCE}.md](../../docs/en/USAGE.md) 以及 [changelog/{0.1.0,unreleased}/en-US.md](../0.1.0/en-US.md)。
- 为无头界面预览工具 `examples/ui_preview/` 与 `scripts/preview.ps1` 新增 `--lang zh|en`（`-Lang`）参数，并生成英文文档预览图 `docs/previews/en/`。
- 新增完全无头、不弹出也不抢占前台窗口的界面预览生成工具（`examples/ui_preview/` 与 `scripts/preview.ps1`），基于 Slint 软件渲染器直接离屏生成主窗口、编辑窗口与设置各分页的浅/深色高清预览图、总览拼图与本地交互式 `index.html` 展示图库（支持精选/全部预设、Window/Card/Raw 边框样式，以及从现有数据目录或导出 JSON 读取真实条目渲染）。
