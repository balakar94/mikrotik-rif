# MikroTik RIF Viewer — 简体中文
# 缺失的标识符取自英文文件。

app-title = MikroTik RIF Viewer

## 欢迎界面
welcome-blurb = 打开 RouterOS 支持包，查看其中包含的所有模块。一切都在你的设备上运行——不会上传任何内容。
welcome-start = 开始

## 主界面
drop-title = 将 supout.rif 拖到这里
drop-title-hover = 松开即可打开文件
drop-hint = 或点击选择文件 · 一切都在你的设备上运行

## 打开动画
phase-reading = 正在读取文件…
phase-indexing = 正在索引模块…
phase-preparing = 正在准备视图…
detail-indexing = 正在读取支持包结构
detail-read-of = { $done } / { $total }
detail-read = 已读取 { $size }
detail-modules-found = 找到 { $count } 个模块

## 状态与底栏
status-ready = 就绪
status-decoding = 正在解码模块…
status-decoding-inline = 正在解码…
status-saved = 已保存到 { $path }
status-indexed = 已索引 { $count } 个模块
status-copied = 已复制 { $count } 字节到剪贴板
footer-build = v{ $version } · { $os } · { $arch }

## 工作区标签
label-modules = 模块
label-line-numbers = 行号
label-no-capture = 未打开支持包
label-compressed = 压缩后 { $size }
label-counter = { $position } / { $total }
label-copy-suffix = · 副本 { $count }
label-find-counter = { $position } / { $total }
empty-selection = 在左侧选择一个模块以查看其内容
empty-filter = 没有模块符合你的搜索

## 输入提示
hint-filter = 筛选模块
hint-find = 在此模块中搜索文本

## 按钮
button-open = 打开…
button-copy = 复制
button-save-as = 另存为…
button-previous = 上一个
button-next = 下一个
button-clear = 清除
button-toggle-rail = 显示或隐藏模块列表
button-find = 在此模块中搜索

## 错误与模块状态
module-unreadable = 无法索引此模块。
error-open = 无法打开 { $path }：{ $reason }
error-module = 无法读取模块 { $index }：{ $reason }
error-save = 无法保存：{ $reason }

## 系统对话框
dialog-open-title = 打开 RouterOS 支持包
dialog-filter-name = RouterOS 支持包

## 更新
update-available = 发现新版本 { $version }
update-check = 检查更新
update-now = 立即更新
update-later = 稍后
update-skip = 跳过此版本
update-auto = 自动检查更新

## 设置
button-settings = 设置
button-settings-update = 设置 — 有新版本 { $version }
button-open-capture = 打开其他支持包…
settings-title = 设置
settings-close = 关闭
settings-tab-general = 常规
settings-tab-updates = 更新
settings-tab-about = 关于
settings-theme = 主题
theme-system = 跟随系统
theme-light = 浅色
theme-dark = 深色
settings-language = 语言
settings-language-system = 系统语言
update-checking = 正在检查更新…
update-up-to-date = 已是最新版本
update-current-version = 版本 { $version }
update-current-build = 构建 { $hash }
update-build-tooltip = 提交 { $commit } · SHA-256 { $hash }
update-downloading = 正在下载更新…
update-verified = 已通过 SHA-256 验证
update-ready = 可以安装
update-install = 下载并安装
update-download-dmg = 下载 .dmg
update-macos-hint = 将应用拖到“应用程序”文件夹即可完成。
update-retry = 重试
update-open-page = 打开发布页面
update-never-checked = 尚未检查
update-last-checked = 上次检查：{ $when }
time-just-now = 刚刚
time-minutes-ago = { $count } 分钟前
time-hours-ago = { $count } 小时前
time-days-ago = { $count } 天前
about-version = 版本 { $version }
about-copyright = Copyright © 2026 balakar94
about-license = 依据 MIT 许可证发布。
about-license-link = 查看许可证
about-repository = GitHub 上的源代码
about-credits-heading = 致谢
about-credits-body = 使用 egui/eframe 及其他开源 Rust crate 构建。内置字体：Inter、JetBrains Mono 和 Noto Sans SC，均依据 SIL Open Font License 1.1 授权；完整的许可证文本及第三方声明随应用一起安装。
about-trademark = MikroTik RIF Viewer 是独立项目，与 MikroTik 无关联，也未获得其认可或赞助。"MikroTik" 和 "RouterOS" 是各自所有者的商标，仅用于描述互操作性。
