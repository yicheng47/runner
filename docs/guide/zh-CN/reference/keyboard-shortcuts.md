# 键盘快捷键

以下为 Runner 默认的 macOS、Windows 与 Linux 快捷键。打开 **设置 → 键盘快捷键（Settings → Keyboard shortcuts）** ，可查看、修改、解除受支持绑定，或恢复默认值。

快捷键名称、说明与命令面板搜索跟随 **设置 → 通用 → 语言（Settings → General → Language）** 。命令面板也接受原始英文命令名称。切换语言保留绑定与已保存覆盖。

macOS 上，`⌘` 为 Command，`⇧` 为 Shift，`⌥` 为 Option，`⌃` 为 Control。操作取决于焦点：窗格快捷键作用于当前窗格，任务导航作用于当前任务。文本字段与智能体终端可能有自己的绑定。

macOS 终端窗格中， **设置 → 终端 → 将 Option 用作 Meta 键（Settings → Terminal → Option as Meta key）** 默认为 **自动（Automatic）** ：在 U.S.、U.S. International 和 ABC 布局中发送 Meta（Esc），在其他布局中输入字符，包括德语的 `@` 与 `[`。 **左右 Option 键（Both Option keys）** 让所有布局都使用 Meta； **仅左 Option 键（Left Option only）** 与 **仅右 Option 键（Right Option only）** 保留另一侧用于字符； **关闭（Off）** 让两侧都输入字符。不产生可打印文本的组合键，包括 Option+Enter、Option+Backspace、Option+方向键和 Ctrl+Option 组合，在所有模式下都保留终端快捷键。Option+←/→ 仍按词移动。Option 输入字符时，Option+Space 输入普通空格，而非不换行空格。Meta 使用不按 Option 时该键输入的字符，因此德语 Option+5 发送 Esc 后跟 `5`。

## 导航与创建

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 新建窗口 | ⇧⌘N | Ctrl+Shift+N |
| 新建标签页：显示启动器页面，开始后才创建标签页 | ⌘N | Ctrl+N |
| 新建终端；填入焦点空窗格或创建标签页 | ⌘T | Ctrl+Shift+T |
| 新建任务：显示启动器页面的小队步骤，不创建标签页 | ⇧⌘M | Ctrl+Shift+M |
| 命令面板 | ⌘K | Ctrl+Shift+P |
| 切换侧边栏 | ⌘S | Ctrl+B |
| 切换任务终端抽屉 | ⌥F12 | Alt+F12 |
| 打开设置 | ⌘, | Ctrl+, |
| 上一个最近查看的页面 | ⇧⌘[ | Ctrl+Shift+[ |
| 下一个最近查看的页面 | ⇧⌘] | Ctrl+Shift+] |
| 选择可见侧边栏行 1–9 | ⌘1–9 | Ctrl+1–9 |

项目视图中，数字快捷键按可见标签页与任务排列，关联分组的任务紧跟在其聊天之后。活动视图中，按智能体和终端两组从上到下排列，跳过折叠的分组及折叠关联分组中的任务；拆分标签页打开需要关注的窗格。启用选择绑定时，按住主修饰键可显示行号。

## 窗格与会话

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 上一个聊天窗格 | ⌘[ | Ctrl+[ |
| 下一个聊天窗格 | ⌘] | Ctrl+] |
| 向右拆分 | ⌘D | Ctrl+Shift+D |
| 向下拆分 | ⇧⌘D | Alt+Shift+D |
| 最大化拆分标签页中的焦点窗格，或恢复拆分 | ⇧⌘↩ | Ctrl+Shift+Enter |
| 隐藏焦点任务抽屉、关闭焦点窗格、聊天标签页或启动器页面 | ⌘W | Ctrl+Shift+W |
| 停止焦点聊天或任务成员 | ⇧⌘X | Ctrl+Shift+X |
| 恢复焦点已停止聊天或任务成员 | ⇧⌘R | Ctrl+Shift+R |
| 上一个任务消息流/会话标签页 | ⌘[ | Ctrl+[ |
| 下一个任务消息流/会话标签页 | ⌘] | Ctrl+] |
| 复制终端选区 | ⌘C | Ctrl+C |
| 粘贴 | ⌘V | Ctrl+V |

Windows 和 Linux 上，终端有选区时 **Ctrl+C** 复制并清除选区，因此下一次 **Ctrl+C** 会作为中断送入终端；没有选区时直接作为中断。 **Ctrl+W**、 **Ctrl+D**、 **Ctrl+K**、 **Ctrl+S** 和 **Ctrl+M** 留给终端程序：Runner 使用 **Ctrl+Shift+W** 关闭窗格或标签页，使用 **Ctrl+Shift+D** 和 **Alt+Shift+D** 拆分，使用 **Ctrl+Shift+P** 打开命令面板，使用 **Ctrl+B** 切换侧边栏。

## 终端中的 Ctrl 键

Windows 和 Linux 上， **设置 → 键盘快捷键 → 终端中的 Ctrl 键（Settings → Keyboard shortcuts → Ctrl keys in terminals）** 决定终端获得焦点时单独的 Ctrl 组合键交给谁。终端包括聊天窗格、终端标签页、任务成员、任务抽屉和远程终端。

- **Runner 优先（Runner first）** （默认）让 Runner 的所有快捷键照常生效，包括在获得焦点的终端中。
- **终端优先（Terminal first）** 把由 Ctrl 加一个键组成、不含 Shift 或 Alt 的快捷键改为发送给获得焦点的终端。按默认键位，这会让出 **Ctrl+N**、 **Ctrl+B**、 **Ctrl+[** / **Ctrl+]**、 **Ctrl+1–9** 和 **Ctrl+,**；你改成单独 Ctrl 组合的快捷键也会让出。这些行显示 **终端外生效（Outside terminals）** 标签，在侧边栏等其他位置仍然有效。

两种模式下，有选区时复制、粘贴和缩放在终端中都有效。设置立即生效。macOS 没有此设置，因为 ⌘ 快捷键不会送入终端。

## 在终端中查找

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 在终端中查找；查找栏已打开时选中查询内容 | ⌘F | Ctrl+Shift+F |
| 下一个（更早的）匹配项 | Enter 或 ⌘G | Enter 或 F3 |
| 上一个（更新的）匹配项 | ⇧Enter 或 ⇧⌘G | Shift+Enter 或 Shift+F3 |
| 关闭查找栏 | Esc | Esc |

可在 **设置 → 键盘快捷键** 中更改“在终端中查找”；查找栏内的按键固定不变。Windows 和 Linux 上， **Ctrl+F** 仍会送入终端。只有查找栏获得焦点时 Esc 才关闭查找栏；终端获得焦点后，Esc 会送给程序。参见[在终端中查找](../guides/chats-and-windows.md#在终端中查找)。

## 侧面板中的文件

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 在预览标签页中打开树中选中的文件 | 空格 | 空格 |
| 在保持打开的标签页中打开树中选中的文件 | Return | Enter |
| 下一个文件标签页 | ⌃Tab | Ctrl+Tab |
| 上一个文件标签页 | ⌃⇧Tab | Ctrl+Shift+Tab |
| 关闭当前文件标签页 | ⌘W | Ctrl+Shift+W |
| 在文件中查找 | ⌘F | Ctrl+F |
| 复制选中的文本 | ⌘C | Ctrl+C |

这些按键在侧面板的文件视图获得焦点时生效，且固定不变。焦点在其他位置或没有打开文件时，⌘W（Ctrl+Shift+W）照常关闭窗格。在文件的查找栏中，Return 和 ⇧Return 在匹配项之间跳转，Esc 关闭查找栏。参见[在侧面板中阅读文件](../guides/chats-and-windows.md#在侧面板中阅读文件)。

## 对话框

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 在对话框中为运行中的任务添加角色 | ⌘Enter | Ctrl+Enter |

对话框确认快捷键为固定绑定。启动器中，↑/↓ 选择条目，Enter 启动所选条目，搜索为空时按 1–4 启动对应条目，在默认页按 Esc 返回之前的标签页或页面；拆分窗格中按 Esc 清空非空搜索。在启动器的小队步骤中，⇧Enter 启动空任务，按 Esc 离开该步骤（搜索中也可），输入框为空时也可按 Backspace 离开。在目标步骤中，Enter 启动任务，⇧Enter 换行，Esc 回到小队列表。搜索中数字正常输入。与其他文本框不同，启动器搜索框中也可使用命令面板快捷键；关闭命令面板后回到搜索框。

## 外观与窗口控制

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 放大 | ⌘+ 或 ⌘= | Ctrl++ 或 Ctrl+= |
| 缩小 | ⌘− | Ctrl+− |
| 重置应用缩放 | ⌘0 | Ctrl+0 |
| 关闭窗口并保存布局 | ⇧⌘W | Alt+F4 |
| 最小化 | ⌘M | 无默认快捷键 |
| 切换全屏 | ⌃⌘F | F11 |
| 退出 Runner | ⌘Q | 关闭最后一个窗口 |
| 退出并停止会话 | ⌥⌘Q | 无默认快捷键 |
| 隐藏 Runner | ⌘H | 无默认快捷键 |
| 隐藏其他应用 | ⌥⌘H | 无默认快捷键 |

应用缩放调整应用控件。终端字号在 **设置 → 终端（Settings → Terminal）** 中单独设置。除显式的 **退出并停止会话（Quit and Stop Sessions）** 外，退出行为遵循 **设置 → 通用 → 退出 Runner 时（Settings → General → When Runner quits）** 。

## 自定义

**设置 → 键盘快捷键** 将快捷键分为 **导航与新建**、 **窗格与会话**、 **任务**、 **终端** 和 **缩放与窗口** 几组；搜索在各组内筛选，没有匹配项的分组会隐藏。选择可编辑快捷键，按下新组合。Runner 检查重叠上下文中的冲突。固定窗口与系统绑定不能重设；标签页选择组可整组解除绑定。使用 **全部恢复默认（Reset all to defaults）** 恢复标准映射。

这些表格依据 Runner 共享键位映射和 Windows/Linux 的 Ctrl 默认值。斜杠命令等智能体命令与终端编辑键属于智能体本身。

在聊天标签页中，停止和恢复快捷键只作用于焦点聊天。终端没有生命周期控制；聊天和终端标签页中的抽屉快捷键不执行操作。
