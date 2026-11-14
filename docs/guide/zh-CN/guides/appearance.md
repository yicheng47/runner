# 外观

选择 Runner 如何跟随浅色与深色模式，再为每种模式选择一个主题：一次选择同时设定应用和终端配色。文字大小与窗口材质各有独立控制。

## 显示语言

打开 **设置 → 通用（Settings → General）** ，使用 **语言（Language）** 下拉框选择 **跟随系统（System）** 、 **English** 或 **简体中文** 。更改会立即应用到所有已打开的 Runner 窗口，包括菜单和搜索标签。草稿、终端内容和运行中的会话都会保留。重新启动后 Runner 会记住选择。

**跟随系统** 是新设置和已有设置的默认值。它读取操作系统的首选界面语言：中文对应简体中文；其他语言或不可用时使用英语。Runner 会在启动、选择跟随系统以及窗口激活时检查。System 的中文标签为 **跟随系统** ；两个显式语言选项保留各自语言的名称。

此设置覆盖 Runner 界面和日期显示。智能体输出、用户自定义名称和提示词、后端错误详情、CLI 输出及文档保留原文。操作系统原生对话框跟随操作系统语言。

## 主题

打开 **设置 → 外观（Settings → Appearance）** 。 **主题（Theme）** 可跟随操作系统，或固定为浅色、深色。下方的 **浅色主题（Light theme）** 和 **深色主题（Dark theme）** 各展示六张主题卡片，点选卡片即可同时设置侧边栏、面板、控件与终端网格；当前生效的模式标题旁会标出 **当前使用（in use now）** ：

- **浅色主题**（“浅色模式下使用”）：Runner Light、Catppuccin Latte、Rosé Pine Dawn、Tokyo Night Day、Gruvbox Light 与 Solarized Light。Runner Light 将偏冷的灰色界面与 Rosé Pine Dawn 的奶油色终端背景搭配使用。
- **深色主题**（“深色模式下使用”）：Runner、Catppuccin Mocha、Rosé Pine Moon、Tokyo Night、Gruvbox Dark 与 Solarized Dark。

为两种模式各选好一次主题后，浅色和深色之间即可直接切换，无需重新组合搭配。已打开的终端会立即更新，Claude Code 也会实时跟随 Runner 的浅色/深色切换；其他智能体自行绘制的颜色可能需使用其自己的主题控制。

Codex 等自行绘制颜色的智能体会在启动时按当前主题的终端配色选择一次浅色或深色样式。Runner 启动时恢复的聊天使用上次的终端配色。如果 **主题** 跟随系统，且 Runner 关闭期间系统切换了浅色/深色，颜色不匹配的聊天需停止再恢复。另一台电脑的终端使用那台电脑的终端配色；见[远程访问](remote-access.md#在应用中配对并打开另一台电脑)。

## 窗口材质

在 macOS 上， **窗口材质（Window material）** 提供 **玻璃（Glass）** 与 **实色（Solid）** 。玻璃为侧边栏和窗口边框增加层次。Runner 遵循 macOS 的 **降低透明度（Reduce Transparency）** ，开启后玻璃会采用实色外观。

Windows 使用实色，不显示 macOS 的材质选择器。

## 界面大小与终端文字

**设置 → 通用 → 界面缩放（Settings → General → App zoom）** 调整应用控件大小。使用 **⌘+** / **Ctrl++** 放大， **⌘−** / **Ctrl+−** 缩小， **⌘0** / **Ctrl+0** 重置。

终端画布有独立文字大小。打开 **设置 → 终端（Settings → Terminal）** ，其中分为 **文字（Text）** 、 **光标（Cursor）** 、 **键盘（Keyboard）** （仅 macOS）和 **回滚与选择（Scrollback & selection）** ，终端配色跟随“外观”中所选的主题。 **字号（Font size）** 范围为 8–32 px， **行高（Line height）** 将行距设为字号的 1.0–1.6 倍；已打开的终端会重新排版，Shell 的行数随之变化。可搜索的 **字体（Font family）** 列表将 JetBrains Mono 放在 **内置（Bundled）** 下，将电脑上的等宽字体放在 **已安装（Installed）** 下。已保存但缺失的字体标为 **未安装（Not installed）**，并使用 JetBrains Mono，直到重新安装该字体或选择其他字体。字体更改会应用到已打开的终端。若侧边栏大小合适而智能体文字太小，请调整终端字号，而非应用缩放。

**光标闪烁（Blink cursor）** 默认关闭，开启后获得焦点的终端光标会闪烁；自行设置光标样式的程序仍由其自己决定。 **回滚行数（Scrollback）** 让每个终端在屏幕上方保留 1,000、5,000、10,000（默认）、25,000 或 50,000 行。行数越多，每个打开的会话占用的内存越多；调低后，已打开终端中最早的行会被丢弃。 **选中即复制（Copy on select）** 默认关闭，开启后用鼠标完成选择时立即将文本复制到剪贴板，内容与 ⌘C 或 Ctrl+C 复制的相同，选区保持不变。

在 macOS 上， **键盘（Keyboard）** 分组中的 **将 Option 用作 Meta 键（Option as Meta key）** 控制 Option 是发送 Shell 快捷键还是输入键盘布局的字符。默认的 **自动（Automatic）** 在 U.S.、U.S. International 和 ABC 布局中使用 Meta，在其他布局中输入字符。可选择 **左右 Option 键（Both Option keys）** 、 **仅左 Option 键（Left Option only）** 、 **仅右 Option 键（Right Option only）** 或 **关闭（Off）** 覆盖默认行为。布局切换从下一次按键生效。
