# 外观

选择 Runner 如何跟随浅色与深色模式，再分别设置应用和终端配色：应用配色在“外观”中设置，终端配色与终端的其他设置放在一起。文字大小与窗口材质各有独立控制。

## 显示语言

打开 **设置 → 通用（Settings → General）** ，使用 **语言（Language）** 下拉框选择 **跟随系统（System）** 、 **English** 或 **简体中文** 。更改会立即应用到所有已打开的 Runner 窗口，包括菜单和搜索标签。草稿、终端内容和运行中的会话都会保留。重新启动后 Runner 会记住选择。

**跟随系统** 是新设置和已有设置的默认值。它读取操作系统的首选界面语言：中文对应简体中文；其他语言或不可用时使用英语。Runner 会在启动、选择跟随系统以及窗口激活时检查。System 的中文标签为 **跟随系统** ；两个显式语言选项保留各自语言的名称。

此设置覆盖 Runner 界面和日期显示。智能体输出、用户自定义名称和提示词、后端错误详情、CLI 输出及文档保留原文。操作系统原生对话框跟随操作系统语言。

## 主题与配色

打开 **设置 → 外观（Settings → Appearance）** 。 **主题（Theme）** 可跟随操作系统，或固定为浅色、深色。每种模式都有两种配色：

- **应用配色（App palette）** 在 **设置 → 外观** 中设置，控制侧边栏、面板与控件。浅色模式提供 Runner Light 和 Catppuccin Latte；深色模式提供 Runner 和 Catppuccin Mocha。Runner Light 使用偏冷的灰色表面与更清晰的边框。
- **终端配色（Terminal palette）** 在 **设置 → 终端 → 配色（Settings → Terminal → Colors）** 中以 **浅色模式配色（Light mode palette）** 和 **深色模式配色（Dark mode palette）** 设置，控制终端网格。Runner、Catppuccin Mocha 和 Rosé Pine Dawn 可独立于应用配色选择。Runner 中的 Rosé Pine Dawn 使用柔和的奶油色背景。外观预览会显示每种模式的两种配色。

为两种模式各选择一次后，就可以直接切换，无需重新组合。Claude Code 会实时跟随 Runner 的浅色/深色切换；其他智能体自行绘制的颜色可能需使用其自己的主题控制。

Codex 等自行绘制颜色的智能体会在启动时按终端配色选择一次浅色或深色样式。Runner 启动时恢复的聊天使用上次的终端配色。如果 **主题** 跟随系统，且 Runner 关闭期间系统切换了浅色/深色，颜色不匹配的聊天需停止再恢复。另一台电脑的终端使用那台电脑的终端配色；见[远程访问](remote-access.md#在应用中配对并打开另一台电脑)。

## 窗口材质

在 macOS 上， **窗口材质（Window material）** 提供 **玻璃（Glass）** 与 **实色（Solid）** 。玻璃为侧边栏和窗口边框增加层次。Runner 遵循 macOS 的 **降低透明度（Reduce Transparency）** ，开启后玻璃会采用实色外观。

Windows 使用实色，不显示 macOS 的材质选择器。

## 界面大小与终端文字

**设置 → 通用 → 界面缩放（Settings → General → App zoom）** 调整应用控件大小。使用 **⌘+** / **Ctrl++** 放大， **⌘−** / **Ctrl+−** 缩小， **⌘0** / **Ctrl+0** 重置。

终端画布有独立文字大小。打开 **设置 → 终端（Settings → Terminal）** ，其中分为 **文字（Text）** 、 **配色（Colors）** 、 **光标（Cursor）** 、 **键盘（Keyboard）** （仅 macOS）和 **回滚与选择（Scrollback & selection）** 。 **字号（Font size）** 范围为 8–32 px， **行高（Line height）** 将行距设为字号的 1.0–1.6 倍；已打开的终端会重新排版，Shell 的行数随之变化。可搜索的 **字体（Font family）** 列表将 JetBrains Mono 放在 **内置（Bundled）** 下，将电脑上的等宽字体放在 **已安装（Installed）** 下。已保存但缺失的字体标为 **未安装（Not installed）**，并使用 JetBrains Mono，直到重新安装该字体或选择其他字体。字体更改会应用到已打开的终端和外观预览。若侧边栏大小合适而智能体文字太小，请调整终端字号，而非应用缩放。

**光标闪烁（Blink cursor）** 默认关闭，开启后获得焦点的终端光标会闪烁；自行设置光标样式的程序仍由其自己决定。 **回滚行数（Scrollback）** 让每个终端在屏幕上方保留 1,000、5,000、10,000（默认）、25,000 或 50,000 行。行数越多，每个打开的会话占用的内存越多；调低后，已打开终端中最早的行会被丢弃。 **选中即复制（Copy on select）** 默认关闭，开启后用鼠标完成选择时立即将文本复制到剪贴板，内容与 ⌘C 或 Ctrl+C 复制的相同，选区保持不变。

在 macOS 上， **键盘（Keyboard）** 分组中的 **将 Option 用作 Meta 键（Option as Meta key）** 控制 Option 是发送 Shell 快捷键还是输入键盘布局的字符。默认的 **自动（Automatic）** 在 U.S.、U.S. International 和 ABC 布局中使用 Meta，在其他布局中输入字符。可选择 **左右 Option 键（Both Option keys）** 、 **仅左 Option 键（Left Option only）** 、 **仅右 Option 键（Right Option only）** 或 **关闭（Off）** 覆盖默认行为。布局切换从下一次按键生效。
