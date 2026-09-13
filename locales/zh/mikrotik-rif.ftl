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
