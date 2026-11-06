# 故障排查

先检查失败的智能体或命令，再查看 Runner 设置与日志。以下步骤会保留已有聊天和任务，帮助你定位问题。

## Runner 无法打开

Runner 在显示窗口前启动后台服务。若失败，会将原因写入 `runner.log`，并在 Windows 上显示 **Runner 无法启动（Runner could not start）** 对话框。日志在 macOS 的 `~/Library/Logs/com.wycstudios.runner` 与 Windows 的 `%APPDATA%\com.wycstudios.runner\logs`。请在[报告问题](#收集有用的错误报告)时附上该信息。

若另一个版本或构建的 Runner 正在使用同一份数据，新打开的 Runner 不会接管后台服务，而是停止并提示另一个 Runner 构建正在使用这些数据。请退出另一个 Runner，再重新打开。若 Runner 打开期间后台服务换了主人，同一提示会以横幅显示：退出另一个 Runner 后点击 **重试（Try again）** 。此检查之前发布的 Runner 版本仍会直接接管后台服务。

## 找不到智能体或无法启动

1. 在常用终端运行智能体。若在那里也无法启动或登录，请先完成它自身的设置。
2. 打开 **设置 → 智能体（Settings → Agents）** ，启用运行时并点击 **刷新（Refresh）** 。若 Runner 未在 `PATH` 中找到，使用 **浏览（Browse）** 选择实际可执行文件。
3. 检查聊天工作目录和模型选择。选择该智能体与账号可用的模型，或使用默认值。
4. 阅读终端中的认证、目录信任或提供商错误。Runner 无法替你完成提供商登录。

Windows 上 npm 启动脚本需要 Node.js，Claude Code 和 pi 的 bash 工具需要 Git for Windows。Cursor、Grok 及其他较新集成的平台验证缺口见[支持的智能体](../reference/supported-agents.md)。运行时可启用并不保证提供商支持所有平台。

## 找不到 runner 命令

打开 **设置 → 通用 → 命令行（Settings → General → Command line）** 查看安装状态。macOS 上必要时使用 **安装（Install）** 。Windows 上 Runner 将命令加入用户 `PATH` 后，请打开新终端。

检查 Shell 找到的命令：

```sh
command -v runner
runner --version
```

PowerShell：

```powershell
Get-Command runner
runner --version
```

如果另一个名为 `runner` 的程序排在前面，请使用设置中显示的路径，或修正 Shell 的 `PATH` 顺序。如果智能体继承了旧环境，请重启会话。

## 沙箱阻止访问 Runner

退出码 `5` 表示调用方沙箱阻止访问本地服务，不表示 Runner 已关闭。请让调用的智能体通过宿主权限机制，在沙箱外重试同一命令。退出码 `3` 时，打开 Runner 后重试。其他失败见[退出码](drive-runner-from-agents.md#退出码)。

## 已停止的对话未恢复

检查 **设置 → 通用 → 启动时恢复运行中的智能体（Settings → General → Resume running agents on launch）** 。只有关闭时记录为运行中的智能体才会自动恢复；手动停止的聊天需要使用 **恢复（Resume）** 。

阅读会话错误和终端输出。恢复需要已保存的对话标识与运行时自己的历史。如果智能体通过原生命令切换对话，Runner 是否能跟踪取决于[运行时](../reference/supported-agents.md)。新聊天不能重建提供商丢失的历史。

任务成员的 **重新启动（Restart）** 会有意开始全新对话。需要连续性时使用 **恢复（Resume）** 。已归档工作需先从 **设置 → 已归档（Settings → Archived）** 还原，再尝试恢复。

## 任务看起来空闲或消息流监控结束

打开成员终端，检查登录、审批或未完成的提示。部分运行时通过终端活动估计工作中与空闲；状态标记并非完成信号。

CLI 监控请运行 `runner mission show <mission-id> --json`，并阅读跟随命令的 stderr。所有会话停止、任务结束或连接错误时，跟随命令都可能退出。如果工作仍在进行，请按监控程序打印的恢复命令继续。恢复已停止任务后，需要重新启动监控。

## MCP 或技能修改未出现

更改注册或全局技能开关后，请启动新会话。这些修改作用于运行时自己的文件；运行中的进程可能已加载配置。检查设置中的路径与所选智能体，外部编辑后使用 **刷新（Refresh）** 。部分技能目录有意不包含内置、项目或插件技能；见 [MCP 服务器与技能](mcp-and-skills.md)。

## 收集有用的错误报告

在 macOS 的 **关于 Runner（About Runner）** ，或各平台的 **设置 → 关于（Settings → About）** 中查看版本。macOS 关于窗口显示应用图标、版本与构建标识，以及版权。发布构建包含提交标识，本地开发构建显示 `dev`。

打开 **设置 → 关于 → 打开日志文件夹（Settings → About → Open log folder）** （macOS 上为 **在访达中显示日志（Reveal logs in Finder）** ）。在 [GitHub issue](https://github.com/yicheng47/runner/issues/new) 中附上 Runner 版本、操作系统、智能体 CLI 及版本、失败步骤与可见错误。去除密钥与私密对话后，仅分享相关日志片段。保留原会话与日志，直到证据已收集；删除应用数据不应是排障第一步。

## Linux 本地构建

源码构建、图形依赖、安装包与 WSL 设置见 [main 上的 Linux 开发说明](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md)。通过 SSH 使用 CLI 时，请先运行 `systemctl --user start runnerd`；远程 CLI 调用不会自动启动服务。最小用户服务会解析账号登录 Shell 以发现 PATH。检查 `systemctl --user status runnerd` 与所选 XDG 数据根目录中的日志目录。

Linux 通知权限由桌面管理。Runner 无法查询通用权限状态；请确认桌面通知守护进程与 portal 后端可用。GPUI 支持剪贴板文本，但不支持剪贴板文件路径。应用更新使用 **设置 → 更新（Settings → Updates）** 。系统 deb 安装需要桌面 polkit 认证代理与 `pkexec`。取消授权、包管理器忙碌或依赖错误会显示在 Runner 中；解决原因后重试。 **设置 → 关于** 显示安装日志。源码构建与非受管理安装不能自我替换；请先安装受管理的包。WSL systemd 服务无法让空闲发行版保持运行。

普通 Linux 窗口可拖动边缘或角调整大小。标题栏中间按钮切换最大化与还原；最大化/全屏窗口没有调整边框。新窗口使用桌面报告的逻辑显示边界。若合成器不报告独立工作区，边界可能包含面板/任务栏区域。报告窗口控制问题时，请注明桌面/会话类型，以及最大化、还原或边缘拖动中哪项失败。
