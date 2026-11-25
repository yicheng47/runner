# 支持的智能体

Runner 启动独立安装的智能体 CLI，并保留其原生终端界面。在 **设置 → 智能体（Settings → Agents）** 中启用智能体、检查检测到的可执行文件，或指定可执行文件覆盖。

Runner 将智能体分为两级支持。**一级支持**的智能体是 Claude Code、Codex、Antigravity CLI、Grok Build 和 pi：Runner 针对它们开发和测试。**社区支持**的智能体是 GitHub Copilot CLI、Cursor、OpenCode v2 和 TRAE CLI：它们可以运行聊天和任务，但 Runner 依靠使用者发现问题，遇到故障请[提交 issue](https://github.com/yicheng47/runner/issues/new)。同一级别内，各智能体的功能仍有差异；下文分别说明每个智能体支持的功能。

## 智能体设置

**设置 → 智能体** 将智能体分为已安装与未安装列表。每行打开对应智能体页面；使用 **智能体（Agents）** 面包屑返回，或点击页眉中的 **打开引导（Open setup）** 在当前窗口重新打开首次引导页。已安装行显示版本、可执行文件、默认标记、启用开关以及可用时的更新按钮；当 `PATH` 中存在 `npm` 时，基于 npm 打包的未安装智能体（Claude Code、Codex、Copilot CLI、pi）行会显示 **安装（Install）**。Tab 与 Shift-Tab 在行和控件间移动；Enter 或 Space 打开焦点行。

智能体页面提供启用开关、 **设为默认（Set as default）** 、检测和更新状态、带浏览及重置的 **可执行文件（Executable）** 字段，以及模型与推理强度的只读 **默认设置（Defaults）** 。Executable 保留 Auto 时自动发现。所选路径应用于使用该智能体默认命令的新会话；自定义命令的角色不变。未安装智能体页面提供安装说明与浏览。

### 启动选项

每个智能体页面都有 **启动（Launch）** 卡片和只读 **命令（Command）** 预览，并包含聊天和任务共用的 **自定义参数（Custom flags）** 字段。切换 **聊天（Chat）** 或 **任务（Mission）** 可查看 Runner 组装的命令，模型、思考力度和首条消息以占位符显示。**复制（Copy）** 复制显示的命令，短暂显示对勾与 **已复制（Copied）** 后恢复为 **复制**，在命令预览不可用时保持禁用。Claude Code 仅在启动时设置了 `CLAUDE_CONFIG_DIR` 的情况下显示该变量；原生账号启动时不额外添加该变量。

| 选项 | 智能体 | 默认值 | 适用范围 |
| --- | --- | --- | --- |
| 跳过审批和沙盒 | Codex | 关闭 | 聊天 |
| 跳过权限确认 | Claude Code | 关闭 | 所有账号的聊天 |
| 跳过权限确认 | Antigravity CLI | 关闭 | 聊天 |
| 远程插件目录 | Codex | 开启 | 聊天、恢复、任务和分叉 |
| TUI 工作区 | Codex | 关闭 | 聊天、恢复和任务 |
| 状态钩子 | Copilot CLI | 开启 | 聊天 |
| 自定义参数 | 所有智能体 | 空 | 聊天、角色、任务、恢复和分叉 |

Bypass 让聊天无需权限确认即可运行；Codex 还会跳过沙盒。仅对信任的文件夹开启。Bypass 开关适用于所有 Codex、Claude Code 和 Antigravity CLI 聊天，包括使用角色的聊天。聊天启动、恢复和分叉都会去除角色已保存的权限参数；关闭 Bypass 时，由智能体自身配置决定审批行为。任务始终使用宽松权限与（受支持运行时的）状态钩子，不受聊天开关影响。

关闭 **远程插件目录** 可跳过 Codex 每次启动时的目录下载，但目录中的插件也将不再加载。Runner 仅为该进程添加 `-c features.remote_plugin=false`，不修改 Codex 配置文件。

保持 **TUI 工作区** 关闭时，Codex 会显示本地 Markdown 文件路径而非打开内置 TUI 侧栏预览，便于在 Runner 的文件侧面板中打开点击的文件链接。Runner 仅为交互式 Codex 启动（聊天、恢复和任务）添加 `-c features.tui_workspace=false`，不修改 Codex 配置文件，也不影响无界面 `codex exec fork`。开启后可使用 Codex 内置的文件预览和侧边窗格。

关闭 **状态钩子** 后，Copilot CLI 会改用终端活动判断状态，不再提供详细等待和结果报告，但仍保留 `SessionStart` 会话标识钩子，确保恢复时打开同一段对话。Codex（0.164.0+）、Claude Code（2.1.295+）、pi（1.1.0+）、Grok（1.0.89+）、OpenCode v2（2.1.0+）与 Antigravity CLI（1.5.8+）在聊天和任务中均原生使用 OSC 7501 终端状态而不挂载状态钩子（Codex、Claude Code 和 Grok 仅保留 `SessionStart` 和 `UserPromptSubmit` 会话标识钩子以及对话记录中的中断/错误信号；OpenCode v2 仅保留对话路由标识 TUI 插件以及 `opencode.db` 中的轮次结果读取；pi 保留 `session_start`、`input`、编辑器草稿与窄 UI 提示等待叠加扩展处理以及对话记录中的中断/错误信号；Antigravity 的会话标题回调独立运行），Cursor 只有会话标识钩子，TRAE 没有钩子，因此这八个运行时均无此开关。

**自定义参数** 会添加到该智能体从下次启动开始的每一次会话中，包括普通聊天、角色聊天、任务、恢复和分叉。Runner 自身的辅助命令（如版本和帮助探测、模型发现、用量读取和无头分叉步骤）不会带上这些参数。按 Enter 或离开输入框保存，按 Escape 恢复已保存的值，清空输入框则移除所有自定义参数。

Runner 在全平台都按 shell 词法拆分 **自定义参数**，且不展开 `~` 或环境变量：空格用于分隔参数，粘贴的换行视为空格，单引号、双引号或反斜杠可将含空格的参数值保持为一项。保存时按以下顺序检查并仅显示第一个问题：引号未闭合或末尾带有单独反斜杠时拒绝保存；与 Runner 自身为该智能体管理的参数冲突时拒绝保存（包括权限参数以及 Codex 的保留 `-c`/`--config` 配置键，但允许其他 `-c` 键和重复的 `--add-dir`）；参数名未出现在 `<command> --help` 中时仍会保存并显示琥珀色警告（打开 Launch 卡片时会重新检查，若无法读取帮助输出则不显示警告）。自定义参数位于前置的恢复子命令之后、角色自身参数之前，因此当 CLI 采用最后出现的值时，角色参数仍会覆盖它。更改在会话下次启动时生效。

## 安装与登录

按各提供商说明设置，再在普通终端运行 CLI 完成认证，之后才在 Runner 启动。

| 智能体 | CLI 运行时名称 | 设置与账号说明 |
| --- | --- | --- |
| Codex | `codex` | [Codex CLI 设置](https://developers.openai.com/codex/cli)；通过 Codex 登录 |
| Claude Code | `claude-code` | [Claude Code 设置](https://code.claude.com/docs/en/setup)；通过 Claude Code 登录 |
| Antigravity CLI | `antigravity` | [Antigravity CLI 参考](https://antigravity.google/docs/cli/reference)；Google 账号；在全平台通过官方发布清单检查新版本并支持在 Runner 中更新（`agy update`）（Windows 上 Runner 仍会在会话与探测期间禁用其后台自动更新以避免终端窗口闪烁，可使用 Runner 的更新操作或运行 `agy update`） |
| pi | `pi` | [pi 设置](https://github.com/earendil-works/pi)；配置模型提供商 |
| GitHub Copilot CLI | `copilot` | [Copilot CLI 设置](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli)；需要 Copilot 订阅 |
| Cursor Agent CLI | `cursor` | [Cursor CLI 设置](https://cursor.com/docs/cli/installation)；需独立于编辑器安装 CLI |
| Grok Build | `grok` | [Grok Build 设置](https://docs.x.ai/build/overview)；使用已有 grok.com 登录；自动更新 |
| OpenCode v2 | `opencode` | [OpenCode v2 文档](https://opencode.ai/v2/docs)；配置可用提供商与模型 |
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
| 智能体钩子报告工作中/空闲 | OSC 7501（0.164.0+） | OSC 7501（2.1.295+） | OSC 7501（1.5.8+） | OSC 7501（1.1.0+） | 是 | 终端活动 | OSC 7501（1.0.89+） |
| OSC 7501 程序状态 | 0.164.0+ | 2.1.295+ | 1.5.8+ | 1.1.0+ | 未声称支持 | 未声称支持 | 1.0.89+ |
| 需要输入报告 | 通过 OSC 7501 报告审批、问题与认证（0.164.0+） | 通过 OSC 7501 报告审批、问题与认证（2.1.295+） | 通过 OSC 7501 报告审批、问题与认证（1.5.8+） | 通过 OSC 7501 报告审批、问题与认证（1.1.0+），另含自定义对话框等待叠加 | 审批与问题 | 否 | 通过 OSC 7501 报告审批、问题与认证（1.0.89+） |
| 从 CLI 发现模型 | 是 | 是 | 是 | 是 | 否 | 是 | 是 |
| 通过设置 → 智能体更新 | 是 | 是 | 是 | 是 | 是 | 否 | 低于最低版本时 |
| 用量指示器 | 是 | 是 | 1.5.8+（macOS 已检查） | 否 | 否 | 否 | 否 |
| 技能 | 目录与开关 | 目录与开关 | 目录 | 目录 | 目录与开关 | 目录 | 目录 |
| 安装 Runner 技能 | 是 | 是 | 是 | 是 | 是 | 是 | 是 |
| 任务权限 | Bypass | Bypass | Bypass | 受信任工作区 | Bypass | Bypass | Bypass |
| 终端渲染有 fixture 覆盖 | 是 | 是 | 是 | 否 | 否 | 否 | 是 |

**终端程序状态** 使用 [OSC 7501](https://www.superlogical.com/rex/docs/build/program-status)。Antigravity CLI 1.5.8+、Claude Code 2.1.295+、pi 1.1.0+、Grok 1.0.89+ 与 OpenCode v2 2.1.0+ 通过探测检测支持；Codex 0.164.0+ 无条件报告。在终端标签页手动启动这些智能体，可通过已有侧边栏和标题栏指示器显示工作中、空闲与需要你。标题栏有空间时，阻塞消息显示在状态详情。Codex、Claude Code、pi、Grok、OpenCode v2 与 Antigravity CLI 聊天与任务均直接以 OSC 7501 作为会话状态来源，报告已完成回复、失败，以及审批、提问和认证等待；不发送 OSC 7501 的会话（例如设置了 `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`、`PI_PROGRAM_STATUS=0`，或 Grok 的 `program_status = "off"`）回退到终端活动。对于带钩子的受管理智能体聊天（Copilot CLI），实时钩子桥接保持状态权威，流报告不能替代更新的钩子状态；终端标签页中的被动状态本身也不会产生未读回复。

**Codex、Claude Code、pi、Grok 与 OpenCode v2 状态与输入关注** 通过共享的程序状态解析器为聊天和任务读取各智能体的原生 OSC 7501 根记录报告，不再安装状态钩子。`working` 覆盖常规轮次、工具调用与上下文压缩；`blocked` 携带 `kind=permission`（**需要审批（Approval needed）**）、`kind=question`（**需要回答（Answer needed）**）或 `kind=auth`/缺省（通用 **需要你处理（Needs you）**）并在等待打开期间暂停小队投递；`done`（对于从不发送 `done` 的 OpenCode v2，则为根记录的 `working` → `idle`）在未聚焦时记录完成未读徽标；`error` 显示 **回复失败（Response failed）**；`idle` 结束当前轮次。对于 Claude Code 与 Grok，带 `id=` 的子记录（后台任务和子智能体）绝不改变工作中、已完成、未读徽标或小队投递，仅补充例如“2 个后台任务”的详情文本。OpenCode v2 会发送一条根汇总记录（`app=opencode`，当单个 TUI 中同时运行多个会话时显示 `N sessions working` 或 `N sessions need input`）以及按会话划分的子记录，子记录绝不改变根状态、未读徽标或小队投递。对于 pi，Runner 的扩展还在 `ui_prompt_start`/`ui_prompt_end`（及 `session_shutdown`）上保留去重于 OSC 7501 `blocked` 之下的窄等待叠加层，使独立打开或叠加在其它提示之上的未标注 `ctx.ui.custom(factory)` 对话框仍显示 **需要你处理** 并暂停小队投递，而原生与已标注的对话框去重为单一等待，且 OSC 7501 仍掌握工作中、已完成、失败和已中断的状态权威。除此之外 Runner 仅保留会话标识与协调钩子（Codex、Claude Code 和 Grok 保留 `SessionStart` 与 `UserPromptSubmit`；OpenCode v2 保留对话路由标识快照；pi 保留 `session_start`、`input` 与原生编辑器草稿跟踪），并结合原生对话记录/存储与取消按键识别 **已中断（Interrupted）** 与 API 错误 **回复失败** 结果（Grok 读取 `updates.jsonl`；OpenCode v2 读取 `opencode.db` 中的 `session_v2.idle_outcome`）。若禁用了 OSC 7501（例如 Claude Code 设置 `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`、pi 设置 `PI_PROGRAM_STATUS=0`，或 Grok 在 `~/.grok/config.toml` 中设置 `program_status = "off"`），会话将回退到终端活动，除 pi 的自定义对话框等待叠加外不声称提供对话框投递保护。

**Antigravity CLI** 会将 `agy models` 中带思考力度后缀的条目合并为单一基础模型（例如 `gemini-4-hydrogen` 或 `claude-opus-5-5`），并通过独立的 **思考强度（Effort）** 选择器列出该模型支持的级别（`low`、`medium`、`high`、`xhigh` 及支持时的 `max`）。由于 Runner 仅在指定 `--model` 时才会传递 `--effort`，当没有显式或继承的启动模型时会禁用 **思考强度**（聊天或小队插槽中留空 **模型（Model）** 覆盖项时仍可继承角色的 **模型**）；选择不支持思考力度的模型或输入旧版带力度后缀的模型 ID（例如 `gemini-4-hydrogen-xhigh` 或 `gpt-oss-120b-medium`）时，也会禁用 **思考强度** 并省略冲突的 `--effort` 参数。

**pi** 聊天标题仅接受以 `π - ` 或 `pi - ` 开头的终端 OSC 标题，防止扩展启动时运行的子进程（例如 `npm view` 或 `cmd.exe` 命令）覆盖聊天名称。在默认 pi 中，`/name` 与 `pi.setSessionName()` 会发送带前缀的 `π - <name> - <cwd>` 标题并自动更新聊天名称；不带 `π - ` 或 `pi - ` 前缀的自定义 `piConfig.name` 前缀与任意 `ctx.ui.setTitle()` 标题会被忽略。

**OpenCode v2** 支持聊天、角色、小队、精确会话标识、动态模型与模型变体、OSC 7501 程序状态（2.1.0+，含审批与问题等待以及 `opencode.db` 失败/中断结果）、技能目录、MCP 设置、Runner 技能以及针对过旧 2.0.x 安装的 `opencode upgrade` 恢复。没有聊天分叉、用量指示器或全局技能开关。后端/原生数据检查覆盖回复、协调与停止/恢复；Windows 检查尚未验证。任务使用其自动权限模式。

**TRAE CLI** 仍需主动启用。提供技能目录、MCP 配置与任务权限支持。不要假定它与主要运行时具有相同功能或平台支持。

## 平台与恢复说明

- Runner 提供 macOS（Apple Silicon 与 Intel）与 Windows x64 构建。上文 Linux 引用描述的是运行时适配器行为，不表示 Runner 的 Linux 构建能力；Linux 测试版安装与验证范围见[安装 Runner](../getting-started/install.md)。
- Windows 智能体原生运行，无需 WSL。Claude Code 和 pi 的 bash 工具需要 Git for Windows。npm 安装的智能体需要 Node.js；Copilot 的 npm 安装需要 Node.js 22。PowerShell 7 可选。
- Antigravity CLI 与 pi 有 Windows 实现路径，但没有完成原生 Windows 冒烟检查。Cursor、Grok 和 OpenCode v2 也存在 Windows 验证缺口。
- Grok 聊天、任务与停止/恢复已在 macOS 检查；完整应用重启持久化仍未验证。在已加载 Grok 仪表盘切换对话，会在下次提示时更新恢复标识，而非空闲时切换就更新。
- 恢复标识用于识别运行时对话，运行时仍须保有该历史。工作中/空闲与需要输入标记依赖运行时报告，不能证明任务完成。

生命周期行为见[会话](../concepts/sessions.md)，检测与恢复问题见[故障排查](../guides/troubleshooting.md)。

## 最低版本与智能体更新

Codex 需要 **0.164.0 或更新版本**、Claude Code 需要 **2.1.295 或更新版本**、Antigravity CLI 需要 **1.5.8 或更新版本**、pi 需要 **1.1.0 或更新版本**、Grok 需要 **1.0.89 或更新版本**、OpenCode v2 需要 **2.1.0 或更新版本**以提供原生 OSC 7501 程序状态（及 Antigravity CLI 的 Unix 用量探测）并配合 Runner 的会话标识钩子。启动器只在过旧智能体的智能体行标记 **需要更新（Update needed）**；角色行不显示更新标签或操作。提示说明启动该智能体或使用它的角色时，会先更新对应 CLI（`codex update`、`claude update`、`agy update`、`pi update`、`grok update`，或针对 OpenCode 2.0.x 的 `opencode upgrade`），窗格显示等待卡片；更新成功后聊天自动启动。恢复或分叉被拒绝时也提供同样的恢复方式。无法确定版本时，Runner 仍允许尝试启动。

当 `PATH` 中仅检测到 OpenCode v1（`0.15.x` 或 `1.x`，缺少 `--standalone`）时，Runner 仍会在 **设置 → 智能体** 中将 **OpenCode v2** 保留在 **未安装（Not installed）** 列表，说明检测到的版本并从启动器的已安装智能体中隐藏。低于 2.0.0 的显式 OpenCode 可执行文件覆盖会被拒绝；因为从 v1 升级到 v2 需要安装 OpenCode v2，被拒绝的 v1 启动或恢复会提供 **打开设置（Open Settings）** 而非更新任务。

在 macOS 和 Linux 上，Antigravity CLI 用量检查同样需要已识别的 **1.5.8 或更新版本**，以确保 Runner 在读取 `/usage` 前可通过非交互方式检查登录状态，避免触发浏览器登录；无法识别版本的构建仍允许尝试启动聊天与任务，但用量会跳过检查并保持不可用。在 Windows 上，Runner 会直接读取 Antigravity 已保存的登录凭据并通过 HTTPS 查询配额接口，不会启动 `agy`。

受支持的智能体有新版本时（npm 打包的 CLI 来自 npm，Antigravity CLI 来自官方发布清单），只有智能体行显示已安装版本、可用版本与 **更新（Update）**，角色行保持原样。你仍可使用受支持的已安装版本启动聊天。侧边栏设置齿轮上的圆点打开 **设置 → 智能体**，覆盖所有已启用且提供更新命令的智能体，包括没有用量仪表的智能体。在 Windows 上，即使 Runner 在会话和探测期间禁用了 `agy` 的后台自动更新，**更新（Update）** 仍会显式运行 `agy update`。

手动安装和更新作为[后台任务](../guides/chats-and-windows.md#后台任务)运行。当 `PATH` 中存在 `npm` 时，Claude Code、Codex、GitHub Copilot CLI 与 pi 可在引导页面或 **设置 → 智能体** 中通过 `npm install -g` 安装；未提供 npm 包的运行时保留手动复制命令与指南链接。再次启动同一任务会加入现有任务。运行中的聊天继续使用原版本，直到重新启动。这些控件用于安装或更新智能体 CLI；Runner 自身的更新仍位于 **设置 → 更新**。智能体自动更新属于另一项功能。
