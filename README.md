<!-- LOGO -->
<h1 align="center">
  <img src="assets/icon.png" alt="Runner" width="128" />
  <br />
  Runner
</h1>

<p align="center">
  <a href="https://github.com/yicheng47/runner/stargazers"><img src="https://img.shields.io/github/stars/yicheng47/runner?style=flat-square&logo=github&label=stars" alt="GitHub stars" /></a>
  <a href="https://github.com/yicheng47/runner/releases"><img src="https://img.shields.io/github/downloads/yicheng47/runner/total?style=flat-square&label=downloads" alt="Downloads" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/yicheng47/runner?style=flat-square&cacheSeconds=86400" alt="License" /></a>
  <a href="#community"><img src="https://img.shields.io/badge/WeChat-user%20group-07C160?style=flat-square&logo=wechat&logoColor=white" alt="WeChat user group" /></a>
  <a href="https://runnersh.dev/downloads/"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS and Windows" /></a>
</p>

<p align="center">
  English · <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <strong>Where terminal agents work together.</strong>
  <br />
  Claude Code, Codex, Antigravity CLI, Copilot CLI and pi on the same task, in one mission. Each agent keeps its own TUI in a real terminal; Runner is what sits between them.
</p>

<p align="center">
  <a href="https://runnersh.dev/"><strong>Official website</strong></a>
  ·
  <a href="https://runnersh.dev/downloads/"><strong>Download Runner</strong></a>
</p>

<p align="center">
  <img src="assets/hero.png" alt="Runner start screen — start a new chat, terminal or crew mission from one workspace" width="100%" />
</p>

<p align="center">
  <a href="#features">Features</a>
  ·
  <a href="#demo">Demo</a>
  ·
  <a href="#supported-agents">Agents</a>
  ·
  <a href="#drive-runner-from-your-agents">CLI</a>
  ·
  <a href="#example-crew">Crew example</a>
  ·
  <a href="./CONTRIBUTING.md">Contributing</a>
</p>

> Alpha · Native macOS (Apple Silicon) and Windows (x64).

Linux ARM64 and x86_64 support is in development, with local tarball, Debian and RPM packaging. Linux downloads have not been published; see [Linux build and install instructions](docs/arch/linux.md) and [validation status](docs/tests/823-linux-support.md).

Runner brings CLI coding agents together in reusable crews, with real terminals and a shared mission feed. Your sessions and history stay on your machine.

## Features

<table>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/crews.png"><img src="assets/crews.png" alt="Crew detail page with coder and reviewer slots, a lead badge, per-slot overrides and shared team conventions" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Reusable crews</h3>
      <p>Compose saved roles into a crew, choose a lead, and define shared team conventions. Review each slot's agent and model, adjust overrides, then launch a mission with the same team whenever you need it.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/mission_feed.png"><img src="assets/mission_feed.png" alt="A mission feed with coder, reviewer and QA coordinating, and a question waiting for your input" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Missions and coordination</h3>
      <p>Each mission gives every crew slot a live terminal and a shared feed where agents coordinate and bring questions to you. Stop, resume or restart one slot without touching the rest. <a href="./docs/arch/arch.md">Architecture →</a></p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/mission_terminal.png"><img src="assets/mission_terminal.png" alt="A mission slot's own terminal: the coder's Codex session, beside the crew's session cards" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Real terminals</h3>
      <p>Each agent keeps its own TUI in a real PTY on a GPU-drawn <code>alacritty_terminal</code> grid, with mouse reporting, IME input (Pinyin included), file-path paste and click-to-open, and a shell drawer beneath every chat and mission.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/chat_split.png"><img src="assets/chat_split.png" alt="One tab split four ways: Codex, Claude Code, Antigravity CLI and pi side by side" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Split tabs and windows</h3>
      <p>Split a tab as far as the window allows, drag panes to reorder, group tabs into folders, and open more windows with <code>⇧⌘N</code> or <code>Ctrl+Shift+N</code>.</p>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="middle">
      <a href="assets/notifications.png"><img src="assets/notifications.png" alt="Settings → Notifications: choose which chat and mission events send desktop notifications" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Desktop notifications</h3>
      <p>While Runner is in the background, get notified when an agent needs input, finishes a turn or fails. Clicking a notification opens its chat or mission. Choose events in <strong>Settings → Notifications</strong>; chat notifications are on by default, and mission notifications are opt-in.</p>
    </td>
  </tr>
</table>

**Also included:**

- **Agents keep working when Runner is closed** — Sessions live in `runnerd`, Runner's background service. Quit the app, or let it crash, and reopen it to the same live terminals; stopped agents resume their own conversations on the next launch.
- **MCP servers and skills in one place** — **Settings → MCP** and **Settings → Skills** list each agent's own config and switch entries off per agent, changing only the entry you touched.
- **Projects** — Bind a working directory once; chats and missions started in it inherit the cwd and group in the sidebar.
- **Light and dark** — Carbon and Runner Light, plus Catppuccin, with an app palette and a terminal palette per mode. Claude Code follows the switch live.

## Demo

A quick tour of reusable roles, crew setup, split terminals, and coder/reviewer/QA coordination in a live mission.

https://github.com/user-attachments/assets/c2209015-1497-403a-98bd-cdc5502bee47

## Prerequisites

Runner runs the agent CLIs you already use; it does not install them. Install and sign in to at least one of Claude Code, Codex, Antigravity CLI, GitHub Copilot CLI or pi before you start, and Runner finds it on your `PATH`. On Windows, Claude Code and pi's bash tool need Git for Windows, and npm-installed CLIs need Node.js.

## Drive Runner from your agents

Everything in Runner is also a `runner` command, so your agents, your scripts and you at a terminal can drive it. Your daily agent can plan a fix, hand it to a coder-and-reviewer crew, and keep working, while every session it started is still a real terminal you can open and watch.

A whole mission, driven from outside the app:

```sh
mission=$(runner mission start --crew <crew> --goal-file - -q < brief.md)
runner mission feed "$mission" --follow                   # the crew's messages as they arrive
runner msg post --mission "$mission" --to <lead> "message"
runner mission answer "$mission" <question_id> <choice>   # answer a question the crew asked you
runner mission stop "$mission"
```

| Commands | What they do |
| --- | --- |
| `runner mission` | Start, follow, answer, stop, resume and archive missions |
| `runner crew`, `runner role`, `runner project` | Create and edit crews, roles and projects |
| `runner chat start`, `runner session` | Start a chat; stop, resume, restart or archive any session |
| `runner msg`, `runner signal`, `runner ask` | How crew members talk to each other inside a mission |
| `runner status`, `runner daemon` | Check and stop Runner's background service |

`runner help` lists every command. Add `--json` for machine-readable output, which is what agents use. Commands start the background service when it is not running, so they work with the app closed.

**Setup.** On macOS, the first launch installs `runner` to `~/.local/bin`, or a writable `/usr/local/bin`, when that directory is already on the login `PATH`; otherwise it is one click in **Settings → General → Command line**. On Windows, Runner adds it to the user `PATH`. Agents need nothing: Runner installs a `runner` skill for every detected agent that points it at the version-matched `runner help agents` guide, and the **Runner skill for agents** switch in the same settings section turns that off.

## Supported agents

| | Codex | Claude Code | Antigravity CLI | pi | GitHub Copilot CLI | Cursor | Grok |
| --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Chats, missions, resume after relaunch | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | macOS ⁵ |
| In-session conversation change updates Runner's resume key | ✓ | ✓ | ✓ | ✓ | — | macOS/Linux only | macOS/Linux only ⁴ |
| Runs on Windows | ✓ | ✓ | ✓ ¹ | ✓ ² | ✓ | — ³ | — ³ |
| Fork a chat | ✓ | ✓ | — | ✓ | — | — | ✓ |
| Working / Idle from the agent's hooks | ✓ | ✓ | macOS only | ✓ | ✓ | — (terminal baseline) | macOS/Linux only |
| Needs you: approval and question dialogs shown | — | ✓ | — | from extensions only | ✓ | — | approval only (macOS/Linux) |
| Model list read from the CLI | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ |
| Update from Settings → Agents | ✓ | ✓ | — | ✓ | ✓ | — | — |
| Weekly usage pill and detailed popover | ✓ | ✓ | ✓ | — | — | — | — |
| Mission access | Bypass | Bypass | Bypass | trusted workspace | Bypass | Bypass | Bypass |
| Skills pane | catalog + on/off | catalog + on/off | catalog | catalog | catalog + on/off | catalog | catalog |
| Runner skill installed | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Terminal rendering covered by fixtures | ✓ | ✓ | ✓ | — | — | — | ✓ |

¹ Antigravity CLI has not been smoke-tested on Windows yet.
² pi runs natively on Windows but has not been smoke-tested there yet; its bash tool requires Git for Windows.
³ Cursor and Grok Build have not been verified on Windows yet.
⁴ Switching between already loaded Grok dashboard sessions updates the key on the next prompt; an idle switch has no native hook.

⁵ Grok chats, missions and stop/resume passed native macOS checks; full app-relaunch persistence remains unverified.

**OpenCode v2** is enabled by default and has been tested in Runner. It supports native PTY chats, roles and crews, exact session IDs, dynamic models, skills and MCP settings; see the [OpenCode spec](./docs/features/archive/592-opencode-runtime.md) and [focused validation](./docs/tests/archive/592-opencode-runtime.md). Fork, usage/quota and update controls are unavailable.

Claude Code, Codex and Antigravity CLI are the primary agents. Claude Code and Codex have tuned launch and nudge timing. GitHub Copilot CLI needs a Copilot subscription. pi brings your own configured model provider. Antigravity CLI signs in with a Google account and updates itself when it starts, so it has no **Update** button. Cursor is enabled by default; see the [Cursor spec](./docs/features/archive/723-cursor-agent-runtime.md) for its current capabilities. Grok Build uses your existing grok.com sign-in, updates itself, and is enabled by default; see the [Grok spec](./docs/features/archive/723-grok-build-runtime.md) and [validation record](./docs/tests/archive/723-grok-build-runtime.md) for platform evidence. TRAE CLI is available as an opt-in runtime in Settings → Agents. [Issues](https://github.com/yicheng47/runner/issues) are welcome.

Runner detects each CLI on `PATH`, with a per-agent executable override in **Settings → Agents**, which also shows each CLI's version and, when a newer one is published, an **Update** button that runs the CLI's own updater in a terminal. PowerShell 7 is optional on Windows. Agents run natively on Windows, without WSL.

## Example crew

The **default Runner shape** is a two-role pair-coding loop: one implements, one reviews, and the loop runs on the working-tree diff until the review is clean — no architect, no dispatch overhead, just the tightest loop that still has a second pair of eyes. Runner seeds this crew on first launch; the source lives in [`examples/pair-coding/`](./examples/pair-coding/).

| Role | Runtime | Responsibility | System prompt |
| --- | --- | --- | --- |
| **@coder** (lead) | `codex` | Branches, implements, runs the checks, hands the diff to the reviewer, fixes findings. | [`coder.md`](./examples/pair-coding/coder.md) |
| **@reviewer** | `codex` | Reads the working-tree diff, reports must-fix issues with file:line pointers, never edits code. | [`reviewer.md`](./examples/pair-coding/reviewer.md) |

The crew's team conventions — feature branch first, review before any commit, nothing merged unless the human asks — are in [`team-conventions.md`](./examples/pair-coding/team-conventions.md). Both slots ship on `codex`; switching `@coder` to `claude-code` makes it a cross-vendor pair, where each model catches what the other's training glosses over.

### More crews

For weirder, more fun crew shapes, peek at [`examples/`](./examples/):

- [`pair-coding/`](./examples/pair-coding/) — the default coder / reviewer pair above
- [`codex-trio/`](./examples/codex-trio/) — the pair above plus a QA slot that builds the reviewed revision and tests it in the live app
- [`dev-crew/`](./examples/dev-crew/) — an architect / impl / reviewer trio: one decomposes, one builds, one audits
- [`docs-crew/`](./examples/docs-crew/) — architect partitions a complex repo, 2+ writers draft per-module docs in parallel, editor harmonizes
- [`tic-tac-toe/`](./examples/tic-tac-toe/) — 2 agents + 1 referee actually playing a game against each other
- [`werewolf/`](./examples/werewolf/) — 6-player social deduction with a god moderator
- [`tomb-raid/`](./examples/tomb-raid/) — a 4-person heist crew run by a DM

Each is a copy-pasteable handle + system-prompt set you can spawn into a new Crew and hit Start.

## Community

- Bugs and feature requests: [GitHub Issues](https://github.com/yicheng47/runner/issues).
- Thanks to [LINUX DO](https://linux.do) for providing a community for discussion and sharing open-source projects.
- 中文用户可以扫码加入 Runner 微信用户群。群二维码 7 天过期，如果扫码提示失效，请[提一个 issue](https://github.com/yicheng47/runner/issues/new) 提醒我更新。

<img src="assets/wechat_group_qr.png" alt="Runner 微信用户群二维码" width="200" />

## Acknowledgements

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** and **[gpui-pre](https://github.com/longbridge/gpui-kit)** — the UI uses a crates.io snapshot of Zed's GPU-accelerated UI framework. Thank you to the Zed team for building and open-sourcing GPUI, and to gpui-kit and Jason Lee (huacnlee) for publishing gpui-pre; Zed's terminal crates were the architectural reference for Runner's terminal split.
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — the terminal grid, parser, and scrollback under every pane.
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — the procedural box-drawing glyph table is transcribed from the WebGL addon under its MIT notice (`crates/runner-app/LICENSE.xterm`).
- **[Windows Terminal ConPTY](https://github.com/microsoft/terminal)** — Windows builds bundle Microsoft's `conpty.dll` and `OpenConsole.exe` under the MIT notice (`crates/runner-app/LICENSE.conpty`).
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## Author

Runner is written and maintained by **Yicheng Wang** (Jason Wang, 王逸成) — [@yicheng47](https://github.com/yicheng47) on GitHub.

## License

MIT. Copyright (c) 2026 Yicheng Wang (Jason Wang). You can use, modify, and redistribute Runner for anything, including at work and inside closed-source products, as long as you keep the copyright notice (see `LICENSE`). Versions released from 2026-08-22 through 2026-09-22 were published under GPL-3.0-only and remain available under that license.
