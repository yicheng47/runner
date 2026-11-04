# 让智能体操作 Runner

使用 `runner` 命令，从 Shell 或另一个智能体启动聊天和任务、跟进工作并发送指令。命令连接 Runner 的本地后台服务，因此应用窗口可以关闭。

## 发现已安装版本

[安装命令](../getting-started/install.md#启用-runner-命令)后运行：

```sh
runner help
runner help agents
runner crew list --json
runner role list --json
```

启用 **设置 → 通用 → 命令行 → 供智能体使用的 Runner 技能（Settings → General → Command line → Runner skill for agents）** 后，Runner 会为检测到的智能体安装技能。技能引导智能体读取 `runner help agents`。委派工作前请让智能体阅读此指南；指令与本地二进制版本匹配。

角色使用唯一代号。小队和项目接受 ID 或精确名称。任务和直接聊天会话接受 ID 或唯一前缀。`--json` 提供结构化输出，`-q` 仅输出结果 ID；两者互斥。

## 启动并跟进小队

从 Shell 启动时，请自行启动任务并跟随其消息流。Runner 聊天中的智能体启动任务时，Runner 会把任务关联到该聊天并向它汇报，见[从 Runner 聊天启动](#从-runner-聊天启动)。任务成员按下文所述接收自己任务的收件箱通知。

编写 UTF-8 编码的 `brief.md`，包含任务、目录假设、验证要求与授权的 Git 操作。在 macOS Shell 中运行：

```sh
mission=$(runner mission start --crew "Pair coding crew" --title "Fix the login bug" --cwd /absolute/path/to/project --goal-file brief.md -q)
runner mission feed "$mission" --follow --json
```

Windows PowerShell：

```powershell
$mission = runner mission start --crew "Pair coding crew" --title "Fix the login bug" --cwd 'C:\Users\you\src\project' --goal-file brief.md -q
if ($LASTEXITCODE -ne 0) { throw 'Mission start failed' }
runner mission feed $mission --follow --json
```

将小队名、标题和目录替换为自己的值。`--goal-file -` 从标准输入读取；使用文件路径可避免不同 Shell 的输入重定向差异。如果项目已指定文件夹，可用 `--project "My app"` 替代 `--cwd`。

跟随命令会显示新事件，直到任务结束或所有会话退出。按 **Ctrl+C** 只结束跟随，不会停止任务。可以在一个终端中保持跟随，另开终端运行其他命令。

## 从 Runner 聊天启动

在 Runner 聊天中，`runner mission start` 会把新任务关联到该聊天。任务行显示在侧边栏中该聊天的下方；任务需要你处理、失败或稳定下来时，Runner 会等聊天空闲且输入框为空，再向聊天输入一行简短的 `[runner]` 汇报。加上 `--no-report` 可只关联、不汇报。从应用、终端或 Runner 外部智能体启动的任务不会关联。

启动任务的智能体会告诉你任务 ID，并说明 Runner 是否会向此聊天汇报：用 `--no-report` 启动、关联到其他聊天或未关联的任务不会汇报到这里；随后它结束当前轮次，而不是监控消息流。收到 `[runner]` 行时，智能体用 `runner mission show` 和 `runner mission feed` 查看任务，告诉你发生了什么，并把待回答的问题转给你。它只按你的决定回答问题。

```sh
runner mission link <mission-id>
runner mission link <mission-id> --chat <session-id> --no-report
runner mission unlink <mission-id>
runner mission list --linked-here
```

`mission link` 把已有任务关联到当前所在的聊天，在聊天外则关联到 `--chat` 指定的聊天；任务已有关联时会改为新聊天。只能关联未归档的智能体聊天，且它须与任务属于同一项目，或两者都不属于项目。`mission unlink` 解除关联。`mission list --linked-here` 列出关联到当前聊天的任务，便于智能体在上下文压缩后重新找到它们。也可以在侧边栏中关联，见[聊天与窗口](chats-and-windows.md#聊天下方的关联任务)。

Runner 聊天之外的智能体没有关联，也收不到汇报。它会告诉你任务 ID，你可以在应用中或用 `runner mission feed <mission-id> --follow` 跟进任务。

## 发送消息并回答问题

使用 `mission show` 查找当前代号、角色与待回答问题。以下示例使用占位符，请替换为实际值；命令语法在 macOS Shell 与 PowerShell 中相同：

```sh
runner mission show <mission-id> --json
runner msg post --mission <mission-id> --to coder "Keep the change limited to the parser."
runner mission answer <mission-id> <question-id> <choice>
runner mission stop <mission-id>
runner mission archive <mission-id>
```

`mission answer` 接受 `mission show` 列出的待回答问题 ID，也接受消息流中该成员 `ask_human` 事件的 ID。其他 ID（包括已回答问题的 ID）会被拒绝，并列出待回答问题的 ID，因此回答不会被静默丢弃。

在任务内，成员可以使用 `runner msg post --to reviewer "Ready for review"` 和 `runner msg read`，无需任务 ID。任务外的写操作需要 `--mission`。外部调用者未使用 `--as` 时以操作者身份发言；不要用 `--as` 冒充其他成员。

Runner 向每个成员推送收件箱通知。等待队友且没有独立工作时，请结束当前轮次。不要启动消息流监控、轮询或睡眠循环来等待消息。团队协作通过后续轮次迭代。有限诊断检查、独立工作，以及为已授权 PR 等待 CI 仍可进行。

使用 `runner msg post --to coder --to reviewer "Please check the parser."` 给多人发消息，或使用开头提及：`runner msg post "@coder @reviewer Please check the parser."`。显式 `--to` 优先。开头收件人标记为 `@handle`，可紧跟一个 `:` 或 `,`，因此 `@coder, @reviewer: Please check.` 会发送给两位成员。代号由小写字母、数字、`_` 和 `-` 组成，以字母或数字开头。任何其他标记都会结束开头提及序列：`@reviewer's audit` 与单独的 `@` 仍是文本，不指定收件人。正文中的提及不指定收件人。未知代号和 `@human` 会被拒绝；有效成员见 `mission show`。所有人都需要行动时才省略收件人以广播；普通跟进请回复发送者。

收到 `[inbox]` 通知时运行 `runner msg read`。它只显示新收件箱消息，随后最多显示十行其他成员定向消息摘要。恢复会话后，或早先消息已不在上下文中时，使用 `runner msg read --all` 查看加入以来的所有小队消息，包括已发送消息及其他人之间的通信。未加过滤的读取只将实际打印的收件箱消息标为已读。`--since <event-id>` 和 `--from <handle>` 即使与 `--all` 合用也不会标为已读，且不显示摘要。默认读取已跟踪未读消息；不要维护 `--since` 游标或用 `--from` 循环消费收件箱。过滤读取后若仍有未读消息，stderr 会报告该次读取时的未读数量，并提示运行未过滤的 `runner msg read`，即使过滤器未返回消息也是如此。stdout 消息输出不变。守护进程不可用时仍能读取。不要发送“收到”等仅确认收悉的回复或待命消息；仍须通过 `runner msg read` 消费收件箱。

成员完成一项任务或有人在等的交接时，会在结束本轮前用 `runner msg post` 发布结果：做了什么、关键验证，以及仍需人类用户或其他成员处理的事项。帖子发给提出请求的一方；若请求来自人类用户，则不加 `--to`。任务的操作者（例如启动任务的聊天）读的是消息流，而不是成员的终端；`idle` 状态只表示一轮结束，因此留在终端里的结果看起来就像成员卡住了。每完成一项任务发一条，这与成员不发送的仅确认收悉和待命消息不同。

## 不使用已保存小队启动

先创建空任务，再逐个添加执行者：

```sh
runner mission start --title "Small task" --cwd /absolute/path/to/project -q
runner mission add-role <returned-mission-id> coder --task-file task.md
runner mission add-role <returned-mission-id> reviewer --task "Review the coder's diff after its handoff."
```

从 Runner 聊天启动的任务会如上文所述关联到该聊天。这些成员收到共享目标、各自任务与收件箱通知。添加和移除角色由操作者控制，任务内的智能体不能操作。完整 [CLI 参考](../reference/cli.md)列出覆盖设置和生命周期命令。

## 退出码

| 代码 | 含义 | 下一步 |
| --- | --- | --- |
| `0` | 成功 | 阅读结果 |
| `1` | Runner 拒绝请求、发生协议错误或监控失败 | 阅读 stderr；监控失败不证明任务已停止 |
| `2` | 用法或引用无效 | 阅读该命令的 `--help`，检查名称或 ID |
| `3` | Runner 未运行或无法作为运行中服务连接 | 打开 Runner 后重试 |
| `5` | 沙箱阻止访问 | 使用宿主权限机制在沙箱外重试同一命令 |

退出码 `5` 无法判断 Runner 是否运行。沙箱中的智能体应先为同一命令请求访问，再诊断应用。
