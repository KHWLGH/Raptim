**简体中文** | [English](en-US.md)

# Unreleased

## 修复

- 修复关闭全局热键后未注销已注册热键、注册表自启读写泄漏 `HKEY`、尾部反斜杠参数引号转义缺陷及任务管理器禁用 `StartupApproved\Run` 后重新启用未清理残留项的问题。
- 修复 `DisplayPolicy` 在防抖阈值变更时未按实时基准重算等待时间、提权启动未遵循会话级“暂停游戏勿扰”开关以及开启暂停后保护态未立即释放的问题。
- 修复贴边收起暂停时 `collapsed_probe` 阻断自动展开逻辑、自绘弹出菜单重入与窗口属性清理遗漏、非 DWM 圆角区域尺寸差 1 像素与 `Raptim.Region` 状态标记问题，并以缓存的 Win32 `EnumDisplayMonitors` 替代多屏遍历避免热插拔异常；同时支持外部文件拖放至 `MainWindow` 前正确还原 `Shortcut`/`Program` 分类与控制字符清理。
- 修复主窗口销毁阶段因 `on_broken_edit` 强持有 `Rc` 引用导致的对象循环泄漏、设置项滑块与下拉框状态失步、`NavTree` 键盘光标滚动绑定单向失效、搜索结果滚动偏移与路径转换详情不可滚动、`EditorWindow` 对网址条目打开时误抓网页图标及错误提示着色异常、`discover-items` 缺失键盘/无障碍语义，以及关闭删除/转换浮层后焦点未回归主视图的问题。
- 修复拖动滚动条时被 `DragGuard` 抑制移动导致轻微卡顿（仅在命中启用 `DragArea` 时启用拖拽防误触）、修复滚屏图标更新与 `interaction_timer` 无条件重置开销，并在鼠标按住持有主窗口 Win32 捕获时暂停贴边自动收起，防止拖出窗口外时触发收回。
- 修复网址参数误识别协议、大小写 `Localhost` 判断、`Program` 工作目录斜杠去重不一致、便携目录迁移后 `dup_key` 过期、`Store::update_targets_checked` 位置对齐错误、`save_icon` 缺 256 KiB 校验、`migrate_delete` 跨层级迁移/200 字符截断、大小写同名分类冲突检测以及保护日志 CSV 转义、表头重复和原生路径前缀脱敏问题。

## 性能与维护

- 将后台 500ms `poll_timer` 改为按需启停（仅在窗口可见、处于保护态或存在待处理文件导入/窗口位置防抖写入时运行），并优化 `FrameClock` 手动重置事件、WinHTTP 会话与缓冲复用、`icons::Cache` 通知合并与队列满短路、`protection` 栈上路径查询与零分配规则匹配、OLE 拖放格式缓存及断链检查免除图标 BLOB 克隆。
- 移除 Slint 1.18 软件渲染器中不绘制像素却放大脏区的冗余 `drop-shadow-*` 属性；主网格改按当前行实际剩余列数实例化图块；将打开数据目录、错误日志与外部链接移至后台线程执行；在 `Store::entries_reusing_icons` 复用图标字节所有权并消除 `tick_window` 空闲状态下的无效系统动画查询与重复重绘。
- 清理未使用的 `ui/icons/export.svg` 与 `import.svg`，为快捷方式条目接入 `shortcut.svg` 兜底图标，并收敛核心库与平台层多余 `pub` 导出、冗余字段（如 `MenuEntry.id`、`ShowSource::Relocate`/`Drop`）及未使用的 UI 绑定、回调与序列化派生。
