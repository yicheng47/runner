<!-- LOGO -->
<h1 align="center">
  <img src="assets/icon.png" alt="Runner" width="128" />
  <br />
  Runner
</h1>

<p align="center">
  <a href="https://github.com/yicheng47/runner/stargazers"><img src="https://img.shields.io/github/stars/yicheng47/runner?style=flat-square&logo=github&label=stars" alt="GitHub stars" /></a>
  <a href="https://github.com/yicheng47/runner/releases"><img src="https://img.shields.io/github/downloads/yicheng47/runner/total?style=flat-square&label=downloads" alt="下载量" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/yicheng47/runner?style=flat-square&cacheSeconds=86400" alt="许可证" /></a>
  <a href="#社区"><img src="https://img.shields.io/badge/WeChat-user%20group-07C160?style=flat-square&logo=wechat&logoColor=white" alt="微信用户群" /></a>
  <a href="https://runnersh.dev/downloads/"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS 与 Windows" /></a>
</p>

<p align="center">
  <a href="./README.md">English</a> · 简体中文
</p>

<p align="center">
  <strong>让终端 agent 协同工作。</strong>
  <br />
  Claude Code、Codex、Antigravity CLI、Copilot CLI 和 pi 在同一个 mission 里做同一件事。每个 agent 在真实终端里保留自己的 TUI，Runner 是夹在它们中间的那一层。
</p>

<p align="center">
  <a href="https://runnersh.dev/"><strong>官方网站</strong></a>
  ·
  <a href="https://runnersh.dev/downloads/"><strong>下载 Runner</strong></a>
</p>

<p align="center">
  <img src="assets/hero.png" alt="Runner 起始页 — 在同一个工作区新建聊天、终端或 crew mission" width="100%" />
</p>

<p align="center">
  <a href="#功能">功能</a>
  ·
  <a href="#演示">演示</a>
  ·
  <a href="#支持的-agent">Agent</a>
  ·
  <a href="#让你的-agent-来驱动-runner">CLI</a>
  ·
  <a href="#示例-crew">Crew 示例</a>
  ·
  <a href="./CONTRIBUTING.md">参与贡献</a>
</p>

> Alpha · 原生支持 macOS（Apple Silicon）和 Windows（x64）。

Linux ARM64 和 x86_64 支持正在开发，提供本地 tarball、Debian 和 RPM 打包路径。Linux 下载尚未发布；请参阅 [Linux 构建与安装说明](docs/arch/linux.md) 和 [验证状态](docs/tests/823-linux-support.md)。

Runner 让命令行编码 agent 通过可复用的 crew、真实终端和共享 mission feed 一起工作。会话和历史记录都保存在你的机器上。

## 功能

<table>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/crews.png"><img src="assets/crews.png" alt="Crew 详情页展示 coder 和 reviewer 槽位、lead 标记、单槽位配置覆盖和共享团队约定" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>可复用的 Crew</h3>
      <p>把保存的角色组合成 crew，指定一个 lead，并设定共享的团队约定。查看各槽位使用的 agent 和模型，按需覆盖配置，之后随时用同一支团队发起 mission。</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/mission_feed.png"><img src="assets/mission_feed.png" alt="mission feed 中 coder、reviewer 和 QA 在协作，一个问题正等待你回答" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Mission 与协作</h3>
      <p>每个 mission 为 crew 的各个槽位提供实时终端和共享 feed，agent 在这里协作，需要你决定的问题也会出现在这里。单个槽位可以单独停止、恢复或重启，不影响其他槽位。<a href="./docs/arch/arch.md">架构 →</a></p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/mission_terminal.png"><img src="assets/mission_terminal.png" alt="mission 里一个槽位自己的终端：coder 的 Codex 会话，旁边是 crew 的会话卡片" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>真实终端</h3>
      <p>每个 agent 都在真实 PTY 里保留自己的 TUI，跑在 GPU 绘制的 <code>alacritty_terminal</code> 网格上：鼠标上报、输入法（包括拼音）、文件路径粘贴和点击打开，每个 chat 和 mission 下面还有一个终端抽屉。</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/chat_split.png"><img src="assets/chat_split.png" alt="一个标签页分成四栏：Codex、Claude Code、Antigravity CLI 和 pi 并排运行" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>分栏与多窗口</h3>
      <p>标签页可以一直分栏到窗口放不下为止，拖动各栏重新排列，把标签页归进文件夹，用 <code>⇧⌘N</code> 或 <code>Ctrl+Shift+N</code> 打开更多窗口。</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/notifications.png"><img src="assets/notifications.png" alt="Settings → Notifications：选择哪些 chat 和 mission 事件发送桌面通知" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>桌面通知</h3>
      <p>Runner 在后台时，agent 需要输入、完成一轮回复或失败都可以发通知，点击后打开对应的 chat 或 mission。在 <strong>Settings → Notifications</strong> 中选择通知事件；chat 通知默认开启，mission 通知需要手动开启。</p>
    </td>
  </tr>
</table>

**更多功能：**

- **Runner 关闭后 agent 照样工作** — 会话跑在 Runner 的后台服务 `runnerd` 里。退出应用或者应用崩溃，重新打开后会接回同一批正在运行的终端；停止的 agent 会在下次启动时各自接着原来的对话。
- **在一个地方管理 MCP 服务与技能** — **Settings → MCP** 和 **Settings → Skills** 按 agent 列出它自己的配置，可以按 agent 单独关掉某一项，只改动你碰过的那一条。
- **项目** — 绑定一次工作目录；在项目里发起的 chat 和 mission 会继承它的 cwd，并在侧边栏里归在一起。
- **浅色与深色** — Carbon 和 Runner Light，外加 Catppuccin，每种模式各有一套应用配色和终端配色。Claude Code 会实时跟着切换。

## 演示

快速展示可复用角色、crew 配置、终端分屏，以及 mission 中 coder、reviewer 和 QA 的协作。

https://github.com/user-attachments/assets/c2209015-1497-403a-98bd-cdc5502bee47

## 前置条件

Runner 运行的是你已经在用的 agent 命令行工具，它不负责安装。开始之前，至少安装 Claude Code、Codex、Antigravity CLI、GitHub Copilot CLI 或 pi 中的一个并完成登录，Runner 会在 `PATH` 上找到它。在 Windows 上，Claude Code 和 pi 的 bash 工具需要 Git for Windows，通过 npm 安装的 CLI 需要 Node.js。

## 让你的 agent 来驱动 Runner

Runner 里的一切也都是一条 `runner` 命令，你的 agent、你的脚本和终端前的你都能用它驱动 Runner。你日常用的 agent 可以规划好一个修复，交给一个 coder 加 reviewer 的 crew 去实现，然后继续干自己的事，而它拉起的每个会话仍然是一个你随时可以打开查看的真实终端。

在应用之外驱动一整个 mission：

```sh
mission=$(runner mission start --crew <crew> --goal-file - -q < brief.md)
runner mission feed "$mission" --follow                   # 实时查看 crew 的消息
runner msg post --mission "$mission" --to <lead> "message"
runner mission answer "$mission" <question_id> <choice>   # 回答 crew 问你的问题
runner mission stop "$mission"
```

| 命令 | 用途 |
| --- | --- |
| `runner mission` | 启动、跟踪、回答、停止、恢复和归档 mission |
| `runner crew`、`runner role`、`runner project` | 创建和编辑 crew、角色和项目 |
| `runner chat start`、`runner session` | 发起 chat；停止、恢复、重启或归档任意会话 |
| `runner msg`、`runner signal`、`runner ask` | mission 内 crew 成员之间的沟通 |
| `runner status`、`runner daemon` | 查看和停止 Runner 的后台服务 |

`runner help` 列出全部命令。加上 `--json` 输出机器可读的结果，agent 用的就是这种。后台服务没有运行时，命令会自己拉起它，所以应用关闭时命令照样可用。

**安装。** 在 macOS 上，如果登录 `PATH` 已包含 `~/.local/bin`，Runner 会在首次启动时把 `runner` 安装到那里；如果 `PATH` 中的 `/usr/local/bin` 可写，则安装到后者；否则去 **Settings → General → Command line** 点一下即可。在 Windows 上，Runner 会把它加入用户 `PATH`。agent 不需要额外设置：Runner 会为每个检测到的 agent 安装 `runner` skill，让它读取与当前版本一致的 `runner help agents` 指南；同一个设置区域里的 **Runner skill for agents** 开关可以关掉它。

## 支持的 Agent

| | Codex | Claude Code | Antigravity CLI | pi | GitHub Copilot CLI | Cursor | Grok |
| --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| 聊天、mission、重启后恢复会话 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | macOS ⁵ |
| 会话内切换对话后更新 Runner 的恢复 ID | ✓ | ✓ | ✓ | ✓ | — | 仅 macOS/Linux | 仅 macOS/Linux ⁴ |
| 在 Windows 上运行 | ✓ | ✓ | ✓ ¹ | ✓ ² | ✓ | — ³ | — ³ |
| 分叉聊天 | ✓ | ✓ | — | ✓ | — | — | ✓ |
| 由 agent 自身的 hook 驱动 Working / Idle 状态 | ✓ | ✓ | 仅 macOS | ✓ | ✓ | —（终端基线） | 仅 macOS/Linux |
| Needs you：显示审批和提问对话框 | — | ✓ | — | 仅来自扩展 | ✓ | — | 仅审批（macOS/Linux） |
| 从 CLI 读取模型列表 | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ |
| 在 Settings → Agents 中更新 | ✓ | ✓ | — | ✓ | ✓ | — | — |
| 侧边栏每周用量条与详细用量弹窗 | ✓ | ✓ | ✓ | — | — | — | — |
| Mission 权限 | Bypass | Bypass | Bypass | 已信任工作目录 | Bypass | Bypass | Bypass |
| Skills 面板 | 目录 + 开关 | 目录 + 开关 | 目录 | 目录 | 目录 + 开关 | 目录 | 目录 |
| 已安装 Runner skill | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 终端渲染有夹具测试覆盖 | ✓ | ✓ | ✓ | — | — | — | ✓ |

¹ Antigravity CLI 尚未在 Windows 上做过冒烟测试。
² pi 在 Windows 上原生运行，但尚未在 Windows 上做过冒烟测试；它的 bash 工具需要 Git for Windows。
³ Cursor 和 Grok Build 尚未在 Windows 上验证。
⁴ 在 Grok dashboard 中切换已加载会话后，下次提交提示才会更新恢复 ID；空闲切换没有原生 hook。

⁵ Grok 的聊天、mission 和停止后恢复通过了 macOS 原生验证；完整重启 app 后的会话恢复尚未验证。

**OpenCode v2** 默认启用，已在 Runner 中通过实测，支持原生 PTY 聊天、角色与 crew，以及精确会话 ID、动态模型、skills 和 MCP 设置。详见 [OpenCode 规格](./docs/features/archive/592-opencode-runtime.md) 和 [针对性验证](./docs/tests/archive/592-opencode-runtime.md)。暂不提供 Fork、用量/配额或更新按钮。

Claude Code、Codex 和 Antigravity CLI 是主要支持的 agent。Claude Code 和 Codex 的启动和催促时序做过调优。GitHub Copilot CLI 需要 Copilot 订阅。pi 使用你已经配置好的模型提供商。Antigravity CLI 使用 Google 账号登录，并在启动时自行更新，所以没有 **Update** 按钮。Cursor 默认启用，当前能力详见 [Cursor 规格](./docs/features/archive/723-cursor-agent-runtime.md)。Grok Build 使用你已有的 grok.com 登录，自行更新，默认启用；各平台证据见 [Grok 规格](./docs/features/archive/723-grok-build-runtime.md) 和 [验证记录](./docs/tests/archive/723-grok-build-runtime.md)。TRAE CLI 可以在 Settings → Agents 中手动启用。欢迎提 [issue](https://github.com/yicheng47/runner/issues)。

Runner 会在 `PATH` 上检测各个 CLI，也可以在 **Settings → Agents** 里为每个 agent 单独指定可执行文件；这里还会显示每个 CLI 的版本，有新版本发布时出现 **Update** 按钮，在终端里运行该 CLI 自带的更新命令。Windows 上 PowerShell 7 可选。agent 在 Windows 上原生运行，不需要 WSL。

## 示例 Crew

**Runner 的默认形态**是一个双角色的结对编程循环：一个实现，一个审查，循环基于工作树 diff 一直跑到审查通过为止。没有架构师，没有派发开销，只有一个仍然保留第二双眼睛的最紧凑的循环。Runner 首次启动时会预置这个 crew，源文件在 [`examples/pair-coding/`](./examples/pair-coding/)。

| 角色（role） | 运行时 | 职责 | 系统提示词 |
| --- | --- | --- | --- |
| **@coder**（lead） | `codex` | 开分支、实现、跑检查，把 diff 交给 reviewer，修复发现的问题。 | [`coder.md`](./examples/pair-coding/coder.md) |
| **@reviewer** | `codex` | 阅读工作树 diff，用 file:line 指出必须修复的问题，从不改代码。 | [`reviewer.md`](./examples/pair-coding/reviewer.md) |

这个 crew 的团队约定（先开功能分支、提交前必须审查、没有人的指示不合并）写在 [`team-conventions.md`](./examples/pair-coding/team-conventions.md)。两个槽位默认都用 `codex`；把 `@coder` 换成 `claude-code` 就成了跨厂商的搭档，各自能抓住对方训练里忽略的东西。

### 更多 Crew

想看更奇特、更有趣的 crew 形态，去 [`examples/`](./examples/) 逛逛：

- [`pair-coding/`](./examples/pair-coding/) — 上面默认的 coder / reviewer 搭档
- [`codex-trio/`](./examples/codex-trio/) — 在上面的搭档基础上加一个 QA 槽位，构建审查通过的版本并在真实应用里测试
- [`dev-crew/`](./examples/dev-crew/) — 架构师 / 实现者 / 审查者三人组：一个拆解，一个构建，一个审计
- [`docs-crew/`](./examples/docs-crew/) — 架构师划分一个复杂仓库，两个以上的写手并行起草各模块文档，编辑统一风格
- [`tic-tac-toe/`](./examples/tic-tac-toe/) — 两个 agent 加一个裁判，真的在互相下棋
- [`werewolf/`](./examples/werewolf/) — 六人狼人杀，由一个上帝主持
- [`tomb-raid/`](./examples/tomb-raid/) — 一支四人盗宝小队，由 DM 主持

每一个都是一套可以直接复制的 handle 加系统提示词，新建一个 Crew 粘进去，点 Start 就能跑。

## 社区

- Bug 和功能建议：[GitHub Issues](https://github.com/yicheng47/runner/issues)。
- 感谢 [LINUX DO](https://linux.do) 社区提供交流与开源推广平台。
- 扫码加入 Runner 微信用户群。群二维码 7 天过期，如果扫码提示失效，请[提一个 issue](https://github.com/yicheng47/runner/issues/new) 提醒我更新。

<img src="assets/wechat_group_qr.png" alt="Runner 微信用户群二维码" width="200" />

## 致谢

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** 与 **[gpui-pre](https://github.com/longbridge/gpui-kit)** — UI 使用 Zed GPU 加速 UI 框架的 crates.io 快照。感谢 Zed 团队构建并开源了 GPUI，感谢 gpui-kit 和 Jason Lee（huacnlee）发布 gpui-pre；Zed 的终端 crate 是 Runner 终端分层的架构参考。
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — 每一栏底下的终端网格、解析器和回滚。
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — 程序化绘制的制表符字形表按其 MIT 声明转录自 WebGL 插件（`crates/runner-app/LICENSE.xterm`）。
- **[Windows Terminal ConPTY](https://github.com/microsoft/terminal)** — Windows 构建按 MIT 声明内置了微软的 `conpty.dll` 和 `OpenConsole.exe`（`crates/runner-app/LICENSE.conpty`）。
- **[Sparkle](https://sparkle-project.org)** — macOS 的更新器。

## 作者

Runner 由 **王逸成**（Yicheng Wang / Jason Wang）编写和维护，GitHub 账号 [@yicheng47](https://github.com/yicheng47)。

## 许可证

MIT。Copyright (c) 2026 Yicheng Wang (Jason Wang)。你可以将 Runner 用于任何用途，包括工作场景和闭源产品，并可自由修改和再分发，只需保留版权声明（见 `LICENSE`）。2026-08-22 至 2026-09-22 期间发布的版本以 GPL-3.0-only 许可证发布，并继续以该许可证提供。
