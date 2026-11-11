# MCP 服务器与技能

在设置中查看智能体可用的工具与指令。Runner 读取各运行时自己的配置；受支持的开关会修改该配置，影响 Runner 内外的新会话。

打开 **设置 → 智能体（Settings → Agents）**，选择智能体后使用 **通用（General）**、**技能（Skills）** 或 **MCP** 标签。仅显示受支持的标签。技能和 MCP 标签显示数量，搜索和刷新只作用于所选智能体。

Claude Code 配置多个账号时，技能和 MCP 标签默认显示主账号。账号下拉框列出所有账号，包括未登录账号；切换后，路径、目录、开关和编辑均跟随所选账号。使用主账号设置的账号会显示共享说明和 **显示主账号（Show main）** 按钮：技能开关同时更改两个账号的共享设置，MCP 目录包含主账号的服务器及所选账号自己的服务器。每次进入页面会重置为主账号。共享设置见 [Claude Code 账号](claude-accounts.md)。

## MCP 服务器

1. 打开 **设置 → 智能体（Settings → Agents）**，选择智能体并打开 **MCP** 标签。
2. 搜索服务器，查看命令或 URL、传输方式与注册信息。
3. 切换所选运行时的注册开关，或打开条目编辑原生 JSON 或 TOML。
4. 启动新会话以读取更改。

Runner 只写入指定名称的服务器条目。为智能体启用服务器时，可以从其他智能体复制受支持的现有定义；禁用则从所选配置注销。若无法复制某种传输方式，请使用智能体自己的工具添加。创建新服务器或重命名也使用智能体自己的工具。同名服务器在不同智能体中可能有不同定义，编辑前请检查所选智能体的条目。

| 运行时 | 默认用户 MCP 配置 |
| --- | --- |
| Codex | `~/.codex/config.toml` |
| Claude Code | `~/.claude.json` |
| Antigravity CLI | `~/.gemini/config/mcp_config.json` |
| GitHub Copilot CLI | `~/.copilot/mcp-config.json` |
| TRAE CLI | `~/.trae/traecli.toml` |
| Grok | `~/.grok/config.toml` |
| OpenCode v2 | `~/.config/opencode/opencode.jsonc` |

Claude Code 的原生主账号使用 `~/.claude.json`。新增的主账号使用 `<account folder>/.claude.json`；技能目录与开关使用 `<account folder>/skills/` 和 `<account folder>/settings.json`。

以上为默认值；运行时环境设置可能改变部分路径。两个平台上的 `~` 均表示用户主目录。实际文件以 Runner 显示的路径为准。Runner 不提供 Cursor 和 pi 的 MCP 管理。

## 技能

打开 **设置 → 智能体（Settings → Agents）**，选择智能体及其 **技能（Skills）** 标签，再选择技能阅读。悬停条目可编辑底层技能文档。在外部修改后，使用 **刷新（Refresh）** 重新加载目录。

以下三种运行时提供全局启用/禁用开关：

| 运行时 | 开关修改的内容 |
| --- | --- |
| Codex | `~/.codex/config.toml` 中的 `[[skills.config]]` 条目 |
| Claude Code | `~/.claude/settings.json` 中的 `skillOverrides` |
| GitHub Copilot CLI | `~/.copilot/settings.json` 中的 `disabledSkills` |

其他受支持运行时可查看目录、阅读和编辑技能文件，但 Runner 不提供全局开关。编辑技能会直接修改共享文件，其他智能体也可能读取它。

目录并非智能体加载的所有指令的完整清单。例如，Codex 系统和插件技能、Claude Code 内置和项目技能、Copilot 项目和插件技能均不列出。所选运行时的说明会解释技能根目录和排除项。

## Runner 自带的技能

**设置 → 通用 → 命令行 → 供智能体使用的 Runner 技能（Settings → General → Command line → Runner skill for agents）** 控制为检测到的智能体安装 Runner 命令指南。任务协调使用捆绑的 `runner` CLI；无需将 Runner 注册为 MCP 服务器即可启动任务。

相关流程见[让智能体操作 Runner](drive-runner-from-agents.md)。若更改未在运行的会话中出现，请参阅[故障排查](troubleshooting.md)。
