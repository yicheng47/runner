# 支持的智能体

Runner 启动独立安装的智能体 CLI，并保留其原生终端界面。在 **设置 → 智能体（Settings → Agents）** 中启用智能体、检查检测到的可执行文件，或指定可执行文件覆盖。

Claude Code 与 Codex 是主要集成，启动与提醒时序经过专门调整。Antigravity CLI 及其他运行时属于次要集成：可运行聊天和任务，但不提供 Runner 的全部功能。

## 智能体设置

**设置 → 智能体** 将智能体分为已安装与未安装列表。每行打开对应智能体页面；使用 **智能体（Agents）** 面包屑返回。已安装行显示版本、可执行文件、默认标记、启用开关，以及可用时的更新按钮。Tab 与 Shift-Tab 在行和控件间移动；Enter 或 Space 打开焦点行。

智能体页面提供启用开关、 **设为默认（Set as default）** 、检测和更新状态、带浏览及重置的 **可执行文件（Executable）** 字段，以及模型与推理强度的只读 **默认设置（Defaults）** 。Executable 保留 Auto 时自动发现。所选路径应用于使用该智能体默认命令的新会话；自定义命令的角色不变。未安装智能体页面提供安装说明与浏览。

### 启动选项

每个智能体页面都有 **启动（Launch）** 卡片和只读 **命令（Command）** 预览。切换 **聊天（Chat）** 或 **任务（Mission）** 可查看 Runner 组装的命令，模型、思考力度和首条消息以占位符显示。**复制（Copy）** 复制显示的命令，短暂显示对勾与 **已复制（Copied）** 后恢复为 **复制**，在命令预览不可用时保持禁用。Claude Code 仅在启动时设置了 `CLAUDE_CONFIG_DIR` 的情况下显示该变量；原生账号启动时不额外添加该变量。

| 选项 | 智能体 | 默认值 | 适用范围 |
| --- | --- | --- | --- |
| 跳过审批和沙盒 | Codex | 关闭 | 聊天 |
| 跳过权限确认 | Claude Code | 关闭 | 所有账号的聊天 |
| 远程插件目录 | Codex | 开启 | 聊天、恢复、任务和分叉 |
| 状态钩子 | Codex、Claude Code、pi、Copilot CLI、OpenCode；macOS/Linux 上的 Grok | 开启 | 聊天 |

Bypass 让聊天无需权限确认即可运行；Codex 还会跳过沙盒。仅对信任的文件夹开启。Bypass 开关适用于所有 Codex 和 Claude Code 聊天，包括使用角色的聊天。聊天启动、恢复和分叉都会去除角色已保存的权限参数；关闭 Bypass 时，由智能体自身配置决定审批行为。任务始终使用宽松权限和状态钩子，不受聊天开关影响。

关闭 **远程插件目录** 可跳过 Codex 每次启动时的目录下载，但目录中的插件也将不再加载。Runner 仅为该进程添加 `-c features.remote_plugin=false`，不修改 Codex 配置文件。

关闭 **状态钩子** 后，Codex（0.163.0+）、Claude Code（2.1.295+）和 pi（1.1.0+）通过终端报告状态，其他智能体根据终端活动判断状态。审批和提问可能统一显示为 **需要你处理**；仅依赖终端活动的智能体无法报告详细等待和结果。会话标识钩子仍然开启，确保恢复时打开同一段对话。Antigravity CLI 原生使用 OSC 7501 终端状态而不挂载状态钩子（其会话标题回调独立运行），Cursor 只有会话标识钩子，TRAE 没有钩子，因此三者均无此开关。更改在会话下次启动时生效。

## 安装与登录

按各提供商说明设置，再在普通终端运行 CLI 完成认证，之后才在 Runner 启动。

| 智能体 | CLI 运行时名称 | 设置与账号说明 |
| --- | --- | --- |
| Codex | `codex` | [Codex CLI 设置](https://developers.openai.com/codex/cli)；通过 Codex 登录 |
| Claude Code | `claude-code` | [Claude Code 设置](https://code.claude.com/docs/en/setup)；通过 Claude Code 登录 |
| Antigravity CLI | `antigravity` | [Antigravity CLI 参考](https://antigravity.google/docs/cli/reference)；Google 账号；启动时自动更新 |
| pi | `pi` | [pi 设置](https://github.com/earendil-works/pi)；配置模型提供商 |
| GitHub Copilot CLI | `copilot` | [Copilot CLI 设置](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli)；需要 Copilot 订阅 |
| Cursor Agent CLI | `cursor` | [Cursor CLI 设置](https://cursor.com/docs/cli/installation)；需独立于编辑器安装 CLI |
| Grok Build | `grok` | [Grok Build 设置](https://docs.x.ai/build/overview)；使用已有 grok.com 登录；自动更新 |
| OpenCode v2 | `opencode` | [OpenCode](https://opencode.ai)；配置可用提供商与模型 |
| TRAE CLI | `trae` | 需主动启用；安装配置 CLI 后在设置 → 智能体中开启 |

除 TRAE 外，列出的运行时被检测到后默认启用。运行时名称是 Runner 标识，不一定等于可执行文件名称。例如 Claude Code 的运行时为 `claude-code`，可执行文件为 `claude`。

## 能力

“是”表示已实现集成。下方平台说明区分实现与原生验证。“目录”表示可显示和编辑技能文档，但没有该运行时的全局技能开关。

| 能力 | Codex | Claude Code | Antigravity CLI | pi | Copilot CLI | Cursor | Grok |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 聊天、任务与已保存对话恢复 | 是 | 是 | 是 | 是 | 是 | 是 | macOS 已检查；重启恢复存在缺口 |
| 原生对话切换更新恢复标识 | 是 | 是 | 是 | 是 | 否 | macOS/Linux 集成 | macOS/Linux 集成 |
| Windows | 是 | 是 | 未验证 | 未验证 | 是 | 未验证 | 未验证 |
| 分叉聊天 | 是 | 是 | 否 | 是 | 否 | 否 | 是 |
| 智能体钩子报告工作中/空闲 | 是 | 是 | OSC 7501（1.5.8+） | 是 | 是 | 终端活动 | macOS/Linux 集成 |
| OSC 7501 程序状态 | 0.163.0+ | 2.1.295+ | 1.5.8+ | 1.1.0+ | 未声称支持 | 未声称支持 | 未声称支持 |
| 需要输入报告 | 通用原生 Needs you（0.163.0） | 审批与问题 | 通过 OSC 7501 报告审批、问题与认证（1.5.8+） | 仅扩展报告 | 审批与问题 | 否 | macOS/Linux 上的审批 |
| 从 CLI 发现模型 | 是 | 是 | 是 | 是 | 否 | 是 | 是 |
| 通过设置 → 智能体更新 | 是 | 是 | 否 | 是 | 是 | 否 | 否 |
| 用量指示器 | 是 | 是 | 1.5.8+（macOS 已检查） | 否 | 否 | 否 | 否 |
| 技能 | 目录与开关 | 目录与开关 | 目录 | 目录 | 目录与开关 | 目录 | 目录 |
| 安装 Runner 技能 | 是 | 是 | 是 | 是 | 是 | 是 | 是 |
| 任务权限 | Bypass | Bypass | Bypass | 受信任工作区 | Bypass | Bypass | Bypass |
| 终端渲染有 fixture 覆盖 | 是 | 是 | 是 | 否 | 否 | 否 | 是 |

**终端程序状态** 使用 [OSC 7501](https://www.superlogical.com/rex/docs/build/program-status)。Antigravity CLI 1.5.8+、Claude Code 2.1.295+ 与 pi 1.1.0+ 通过探测检测支持；Codex 0.163.0+ 无条件报告。在终端标签页手动启动这些智能体，可通过已有侧边栏和标题栏指示器显示工作中、空闲与需要你。标题栏有空间时，阻塞消息显示在状态详情。Antigravity CLI 聊天与任务直接以 OSC 7501 作为会话状态来源，报告已完成回复、失败，以及审批、提问和认证等待；不支持 OSC 7501 的旧版本回退到终端活动。对于带钩子的受管理智能体聊天，实时钩子桥接保持状态权威，流报告不能替代更新的钩子状态；终端标签页中的被动状态本身也不会产生未读回复。启动卡片可关闭带钩子聊天的状态钩子；会话标识钩子仍保留，活动状态改由终端提供。

**Codex 输入关注** 使用在 Codex CLI 0.163.0 验证的原生 OSC 7501 报告。`blocked` 报告在现有会话、侧边栏、标题栏与后台通知中显示通用的 **需要你处理（Needs you）** 。Codex 不区分提示是审批还是问题。原生 `working`、`idle` 或 `clear` 会解除关注；仅输入文字不会。钩子仍报告工作、完成、中断与对话标识。原生等待打开时也会暂停小队投递，解除后保留独立草稿。旧版或自定义 Codex 会话若无可用原生报告，保留原状态行为，不声称提供对话框投递保护。此信号在 macOS 与 Windows 的原生应用验证仍待完成。

**Antigravity CLI** 会将 `agy models` 中带思考力度后缀的条目合并为单一基础模型（例如 `gemini-4-hydrogen` 或 `claude-opus-5-5`），并通过独立的 **思考强度（Effort）** 选择器列出该模型支持的级别（`low`、`medium`、`high`、`xhigh` 及支持时的 `max`）。由于 Runner 仅在指定 `--model` 时才会传递 `--effort`，当没有显式或继承的启动模型时会禁用 **思考强度**（聊天或小队插槽中留空 **模型（Model）** 覆盖项时仍可继承角色的 **模型**）；选择不支持思考力度的模型或输入旧版带力度后缀的模型 ID（例如 `gemini-4-hydrogen-xhigh` 或 `gpt-oss-120b-medium`）时，也会禁用 **思考强度** 并省略冲突的 `--effort` 参数。

**OpenCode v2** 支持聊天、角色、小队、精确会话标识、动态模型与模型变体、钩子状态、技能目录、MCP 设置和 Runner 技能。没有聊天分叉、用量指示器、全局技能开关或更新控制。后端/原生数据检查覆盖回复、协调与停止/恢复；完整 GUI 和 Windows 检查尚未验证。任务使用其自动权限模式。

**TRAE CLI** 仍需主动启用。提供技能目录、MCP 配置与任务权限支持。不要假定它与主要运行时具有相同功能或平台支持。

## 平台与恢复说明

- Runner 提供 macOS（Apple Silicon 与 Intel）与 Windows x64 构建。上文 Linux 引用描述的是运行时适配器行为，不表示 Runner 的 Linux 构建能力；Linux 测试版安装与验证范围见[安装 Runner](../getting-started/install.md)。
- Windows 智能体原生运行，无需 WSL。Claude Code 和 pi 的 bash 工具需要 Git for Windows。npm 安装的智能体需要 Node.js；Copilot 的 npm 安装需要 Node.js 22。PowerShell 7 可选。
- Antigravity CLI 与 pi 有 Windows 实现路径，但没有完成原生 Windows 冒烟检查。Cursor、Grok 和 OpenCode 也存在 Windows 验证缺口。
- Grok 聊天、任务与停止/恢复已在 macOS 检查；完整应用重启持久化仍未验证。在已加载 Grok 仪表盘切换对话，会在下次提示时更新恢复标识，而非空闲时切换就更新。
- 恢复标识用于识别运行时对话，运行时仍须保有该历史。工作中/空闲与需要输入标记依赖运行时报告，不能证明任务完成。

生命周期行为见[会话](../concepts/sessions.md)，检测与恢复问题见[故障排查](../guides/troubleshooting.md)。

## 最低版本与智能体更新

Codex 需要 **0.149.0 或更新版本**才能运行 Runner 的启动钩子。启动器只在旧版 Codex 的智能体行标记 **需要更新（Update needed）**；角色行不显示更新标签或操作。提示说明启动 Codex 或使用它的角色时，会先更新 Codex，窗格显示等待卡片；更新成功后聊天自动启动。恢复或分叉被拒绝时也提供同样的恢复方式。无法确定版本时，Runner 仍允许尝试启动。

Antigravity CLI 用量检查需要 **1.5.8 或更新版本**，以确保 Runner 在读取 `/usage` 前可通过非交互方式检查登录状态，避免触发浏览器登录。旧版或无法识别版本的 Antigravity CLI 仍可正常用于聊天与任务，但用量会跳过检查并保持不可用。

受支持的智能体有新版本时，只有智能体行显示已安装版本、可用版本与 **更新（Update）**，角色行保持原样。你仍可使用受支持的已安装版本启动聊天。侧边栏设置齿轮上的圆点打开 **设置 → 智能体**，覆盖所有已启用且提供更新命令的智能体，包括没有用量仪表的智能体。

手动更新作为[后台任务](../guides/chats-and-windows.md#后台任务)运行。再次启动同一智能体的更新会加入现有任务。运行中的聊天继续使用原版本，直到重新启动。这些控件更新智能体 CLI；Runner 自身的更新仍位于 **设置 → 更新**。智能体自动更新属于另一项功能。
