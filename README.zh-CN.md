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
  <a href="#下载"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS 与 Windows" /></a>
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
  <img src="assets/hero.png" alt="Runner — mission 的事件 feed：coder、reviewer 和 QA 三个 agent 围绕同一个目标协作，lead 的问题在等你回答" width="100%" />
</p>

<p align="center">
  <a href="#关于">关于</a>
  ·
  <a href="#功能">功能</a>
  ·
  <a href="#支持的-agent">Agent</a>
  ·
  <a href="#让你的-agent-来驱动-runner">CLI</a>
  ·
  <a href="#示例-crew">Crew 示例</a>
  ·
  <a href="#下载">下载</a>
  ·
  <a href="./AGENTS.md">参与贡献</a>
</p>

> 状态：alpha，持续迭代中。原生支持 macOS（Apple Silicon）和 Windows（x64），Windows 从 0.8.0 起加入。

## 关于

Runner 是一个原生桌面应用，用来让命令行编码 agent **一起**干活。同时跑多个 agent 本来就不难——一人一个终端，它们就能并行，各干各的。Runner 要解决的是另一件事：同一个任务、不同的角色、一条共享的 feed，以及一个在该你拍板时把你叫进来的 lead。对工作流有主张，对厂商保持中立：coder 可以是 Claude Code，reviewer 可以是 Codex。

- **角色（role）** — 一份可复用的 agent 配置：运行时、系统提示词、工作目录。
- **Crew** — 把若干角色组合成有名字的槽位，指定一个 lead，再加上每个 mission 都会继承的团队约定。
- **Mission** — 一个 crew 围绕一个目标干活：每个槽位一个实时终端，通过一条可持久化、可回放的事件 feed 协作，需要你拍板时用 `ask_human` 提问。
- **Chat** — 单个 agent 跑在一个真实终端里，不需要 mission；标签页可以一直分栏到窗口放不下为止。
- **CLI** — 上面的一切也都有对应命令，agent、脚本和终端前的人都可以自己驱动 Runner。

用 Rust 写成，通过 [gpui-pre](https://github.com/longbridge/gpui-kit) 使用 [Zed](https://zed.dev) 的 GPUI，终端网格用 `alacritty_terminal`，状态存在 SQLite。没有 webview。会话由后台服务 `runnerd` 持有，所以应用关闭后 agent 照样继续工作；应用和 `runner` 命令都是它的客户端。一切都在你自己的机器上运行和保存。

## 前置条件

Runner 运行的是你已经在用的 agent 命令行工具，它不负责安装。开始之前，至少安装 Claude Code、Codex、Antigravity CLI、GitHub Copilot CLI 或 pi 中的一个并完成登录，Runner 会在 `PATH` 上找到它。在 Windows 上，Claude Code 和 pi 的 bash 工具需要 Git for Windows，通过 npm 安装的 CLI 需要 Node.js。

## 下载

在[下载页](https://runnersh.dev/downloads/)获取最新版本：macOS（Apple Silicon）是已签名并完成公证的 `.dmg`，Windows 10 1809 或更高版本是已签名的 `Runner-Setup-…-x64.exe` 安装包。Intel Mac、Windows ARM64 和 Linux 暂不支持。

两个平台都支持原地更新，macOS 走 Sparkle，Windows 走 Settings 旁边的更新图标，设置、对话和 mission 都会保留。更新会重启正在运行的 agent，每个 agent 都会接着自己原来的对话。在 Windows 上，证书积累信誉之前，新版本可能仍会触发 SmartScreen 警告，点 **更多信息 → 仍要运行** 即可继续。

## 演示

快速展示可复用角色、crew 配置、终端分屏，以及 mission 中 coder、reviewer 和 QA 的协作。

https://github.com/user-attachments/assets/c2209015-1497-403a-98bd-cdc5502bee47

## 功能

每项功能的界面演示见 [runnersh.dev](https://runnersh.dev/)。

- **Crew 与 mission** — crew 把角色放进有名字的槽位，指定一个 lead，并带上共同的团队约定；mission 为每个槽位开一个实时终端，再加一条 feed，crew 在这里协作，问你的问题也在这里等你。单个槽位可以单独停止、恢复或重启，不影响其他槽位。[架构 →](./docs/arch/arch.md)

  <img src="assets/mission_terminal.png" alt="mission 里一个槽位自己的终端：coder 的 Codex 会话，旁边是 crew 的会话卡片" width="100%" />

- **真实终端** — 每个 agent 都在真实 PTY 里保留自己的 TUI，跑在 GPU 绘制的 `alacritty_terminal` 网格上：鼠标上报、输入法（包括拼音）、文件路径粘贴和点击打开，每个 chat 和 mission 下面还有一个终端抽屉。
- **Runner 关闭后 agent 照样工作** — 会话跑在 Runner 的后台服务 `runnerd` 里。退出应用或者应用崩溃，重新打开后会接回同一批正在运行的终端；停止的 agent 会在下次启动时各自接着原来的对话。
- **分栏与多窗口** — 标签页可以一直分栏到窗口放不下为止，拖动各栏重新排列，把标签页归进文件夹，用 `⇧⌘N` 或 `Ctrl+Shift+N` 打开更多窗口。

  <img src="assets/chat_split.png" alt="一个标签页分成四栏：Codex、Claude Code、Antigravity CLI 和 pi 并排运行" width="100%" />

- **在一个地方管理 MCP 服务与技能** — **Settings → MCP** 和 **Settings → Skills** 按 agent 列出它自己的配置，可以按 agent 单独关掉某一项，只改动你碰过的那一条。
- **项目** — 绑定一次工作目录；在项目里发起的 chat 和 mission 会继承它的 cwd，并在侧边栏里归在一起。
- **浅色与深色** — Carbon 和 Runner Light，外加 Catppuccin，每种模式各有一套应用配色和终端配色。Claude Code 会实时跟着切换。

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

| | Codex | Claude Code | Antigravity CLI | pi | GitHub Copilot CLI | Cursor |
| --- | :---: | :---: | :---: | :---: | :---: | :---: |
| 聊天、mission、重启后恢复会话 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 会话内切换对话后更新 Runner 的恢复 ID | ✓ | ✓ | ✓ | ✓ | — | 仅 macOS/Linux |
| 在 Windows 上运行 | ✓ | ✓ | ✓ ¹ | ✓ ² | ✓ | — ³ |
| 分叉聊天 | ✓ | ✓ | — | ✓ | — | — |
| 由 agent 自身的 hook 驱动 Working / Idle 状态 | ✓ | ✓ | 仅 macOS | ✓ | ✓ | —（终端基线） |
| Needs you：显示审批和提问对话框 | — | ✓ | — | 仅来自扩展 | ✓ | — |
| 从 CLI 读取模型列表 | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| 在 Settings → Agents 中更新 | ✓ | ✓ | — | ✓ | ✓ | — |
| 侧边栏每周用量条与详细用量弹窗 | ✓ | ✓ | ✓ | — | — | — |
| Mission 权限 | Bypass | Bypass | Bypass | 已信任工作目录 | Bypass | Bypass |
| Skills 面板 | 目录 + 开关 | 目录 + 开关 | 目录 | 目录 | 目录 + 开关 | 目录 |
| 已安装 Runner skill | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 终端渲染有夹具测试覆盖 | ✓ | ✓ | ✓ | — | — | — |

¹ Antigravity CLI 尚未在 Windows 上做过冒烟测试。
² pi 在 Windows 上原生运行，但尚未在 Windows 上做过冒烟测试；它的 bash 工具需要 Git for Windows。
³ Cursor 尚未在 Windows 上验证。

Claude Code、Codex 和 Antigravity CLI 是主要支持的 agent。Claude Code 和 Codex 的启动和催促时序做过调优。GitHub Copilot CLI 需要 Copilot 订阅。pi 使用你已经配置好的模型提供商。Antigravity CLI 使用 Google 账号登录，并在启动时自行更新，所以没有 **Update** 按钮。Cursor 默认关闭，当前能力详见 [Cursor 规格](./docs/features/723-cursor-agent-runtime.md)。欢迎提 [issue](https://github.com/yicheng47/runner/issues)。

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
- [`dev-crew/`](./examples/dev-crew/) — 架构师 / 实现者 / 审查者三人组：一个拆解，一个构建，一个审计
- [`docs-crew/`](./examples/docs-crew/) — 架构师划分一个复杂仓库，两个以上的写手并行起草各模块文档，编辑统一风格
- [`tic-tac-toe/`](./examples/tic-tac-toe/) — 两个 agent 加一个裁判，真的在互相下棋
- [`werewolf/`](./examples/werewolf/) — 六人狼人杀，由一个上帝主持
- [`tomb-raid/`](./examples/tomb-raid/) — 一支四人盗宝小队，由 DM 主持

每一个都是一套可以直接复制的 handle 加系统提示词，新建一个 Crew 粘进去，点 Start 就能跑。

## 社区

- Bug 和功能建议：[GitHub Issues](https://github.com/yicheng47/runner/issues)。
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
