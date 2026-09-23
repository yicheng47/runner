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
  <a href="#download"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS and Windows" /></a>
</p>

<p align="center">
  English · <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <strong>Where terminal agents work together.</strong>
  <br />
  Claude Code, Codex, Copilot CLI and pi on the same task, in one mission. Each agent keeps its own TUI in a real terminal; Runner is what sits between them.
</p>

<p align="center">
  <a href="https://github.com/yicheng47/runner/releases/latest"><strong>Download Runner</strong></a>
</p>

<p align="center">
  <img src="assets/hero.png" alt="Runner — a mission feed with three agents on one goal, and a second window splitting four live agent terminals" width="100%" />
</p>

<p align="center">
  <a href="#about">About</a>
  ·
  <a href="#features">Features</a>
  ·
  <a href="#supported-agents">Agents</a>
  ·
  <a href="#mcp-servers-and-skills-managed-in-one-place">MCP</a>
  ·
  <a href="#drive-runner-from-your-agents">CLI</a>
  ·
  <a href="#example-crew">Crew example</a>
  ·
  <a href="#download">Download</a>
  ·
  <a href="./AGENTS.md">Contributing</a>
</p>

> Status: alpha, actively shipping. Native macOS (Apple Silicon) and Windows (x64), with Windows support starting in 0.8.0.

## About

Runner is a native desktop app for running CLI coding agents **together**. Running several at once is already easy — give each one a terminal and they work in parallel, isolated from one another. Runner is for the other thing: one task, different roles, a shared feed, and one lead who pulls you in when the decision is yours. It is opinionated about the workflow and neutral about the provider, so your coder can be Claude Code and your reviewer Codex.

- **Role** — a reusable agent configuration: runtime, system prompt, working directory.
- **Crew** — roles composed into named slots with one lead, plus the team conventions every mission inherits.
- **Mission** — a crew working one goal: one live terminal per slot, coordinating over an event feed that persists and replays, with `ask_human` when a decision is yours.
- **Chat** — a single agent in a real terminal, no mission required; split a tab as far as the window allows.
- **CLI** — everything above is a command too, so agents, scripts, and people at a terminal can drive Runner themselves.

Written in Rust on [gpui-ce](https://github.com/gpui-ce/gpui-ce), the community fork of [Zed](https://zed.dev)'s GPUI, with `alacritty_terminal` for the grid and SQLite for state. No webview. Everything runs and persists on your machine.

## Download

Grab the latest build from the [releases page](https://github.com/yicheng47/runner/releases/latest): a signed and notarized `.dmg` for macOS on Apple Silicon, and a signed `Runner-Setup-…-x64.exe` installer for Windows 10 version 1809 or later. Intel Macs, Windows ARM64, and Linux are not supported.

Both platforms update in place, macOS through Sparkle and Windows through the update icon beside Settings, and keep your settings, chats, and missions. On Windows, SmartScreen may still warn on a fresh release while the certificate builds reputation; **More info → Run anyway** continues.

## Community

- Bugs and feature requests: [GitHub Issues](https://github.com/yicheng47/runner/issues).
- 中文用户可以扫码加入 Runner 微信用户群。群二维码 7 天过期，如果扫码提示失效，请[提一个 issue](https://github.com/yicheng47/runner/issues/new) 提醒我更新。

<img src="assets/wechat_group_qr.png" alt="Runner 微信用户群二维码" width="200" />

## Demo

A three-agent crew — two players and a referee — playing a game of tic-tac-toe against each other over the mission feed, from the [`tic-tac-toe`](./examples/tic-tac-toe/) example crew.

https://github.com/user-attachments/assets/fb3669a4-010d-42d0-9555-2a3ba3223c75

## Features

<table>
<tr>
<td width="60%">
  <img src="assets/roles.png" alt="Roles — reusable agent configurations, each with its runtime, command, and the crews and sessions using it" width="100%" />
  <img src="assets/crews.png" alt="Crews — codex-crew and claude crew, each a coder lead and a reviewer drawn from roles" width="100%" />
</td>
<td width="40%" valign="middle">

### Crews — roles, prompts, one lead

A **role** is a reusable agent configuration: runtime, system prompt, working directory. A **crew** composes roles into named slots with exactly one lead, plus team conventions and a definition of done that every mission inherits.

</td>
</tr>
<tr>
<td width="60%">
  <img src="assets/mission_feed.png" alt="Mission workspace — the feed, where three agents and the human coordinate one goal" width="100%" />
  <img src="assets/mission_terminal.png" alt="Mission workspace — the coder's slot, a live Codex terminal mid-task" width="100%" />
</td>
<td width="40%" valign="middle">

### Missions — a crew working one goal

Starting a mission spawns one live PTY per slot into a tabbed workspace. The **feed** is where the crew coordinates: an append-only event log, every signal persisted and replayable, so missions survive a quit or crash, and `ask_human` questions surface there for you. Each **slot** is a real terminal one tab over, the agent's own TUI, where you can watch, type, or stop, resume, and restart that session on its own.

[Architecture →](./docs/arch/arch.md)

</td>
</tr>
<tr>
<td width="60%">
  <img src="assets/chat_split.png" alt="Chat tab with four split panes and organized sidebar" width="100%" />
  <img src="assets/chat_drag.png" alt="Dragging a pane by its grip — the highlighted half of the target shows where it lands" width="100%" />
</td>
<td width="40%" valign="middle">

### Chats — tabs, split panes, folders

Every chat is a real 1:1 PTY with a role, no mission required. Split a tab as far as the window allows — right or down from any pane, `⌘D` and `⇧⌘D` — and drag a pane by its grip to reorder; terminal tabs split the same way, straight into another shell. The sidebar groups tabs into collapsible folders; every tab shows a spinner while a pane is still working and a dot when one finished while you were elsewhere, so a wall of parallel agents stays scannable.

</td>
</tr>
<tr>
<td width="60%">
  <img src="assets/multi_window.png" alt="Two Runner windows, a Claude Code chat in front and a Codex chat behind" width="100%" />
</td>
<td width="40%" valign="middle">

### Multi-window

`⇧⌘N` on macOS or `Ctrl+Shift+N` on Windows opens additional OS windows — a mission on one screen, a wall of chats on the other. Windows coordinate ownership of shared sessions: the primary owns the PTY, and any other window showing the same session gets a hand-off overlay instead of a corrupted terminal.

</td>
</tr>
<tr>
<td width="60%">
  <img src="assets/skills.png" alt="Settings → Skills — every skill an agent can load, with a toggle per skill" width="100%" />
</td>
<td width="40%" valign="middle">

### MCP servers and skills, managed in one place

Each agent keeps its MCP servers and its skills in its own config files. **Settings → MCP** and **Settings → Skills** read those files and show them as one list per agent: pick Claude Code or Codex, see everything it has, flip a toggle to switch a server or a skill off for that agent's new sessions, click a skill to read it or edit it. Runner changes only the one entry you touched and leaves the rest of the file exactly as you wrote it.

</td>
</tr>
<tr>
<td width="60%">
  <img src="assets/light.png" alt="Runner in Runner Light — a Claude Code chat on the light theme" width="100%" />
  <img src="assets/appearance.png" alt="Settings → Appearance — light and dark previews with an app and a terminal palette per mode" width="100%" />
</td>
<td width="40%" valign="middle">

### Light and dark, designed for Runner

Carbon and Runner Light are Runner's own themes; Catppuccin Mocha and Latte ride along. Auto follows the OS, Light and Dark pin it, and a Claude Code chat follows the flip the moment it happens, no restart, no `/theme`.

### A palette per mode, previewed

**Settings → Appearance** picks an app palette and a terminal palette for light and for dark above a live preview of both modes, so a light app gets a light terminal without a second setting. Rosé Pine Dawn is the default light terminal.

</td>
</tr>
</table>

### Also in the box

- **Sessions that outlive the app** — quitting or crashing does not kill your agents; the next launch reattaches to the sessions still running, and a quit while work is in flight asks first.
- **Projects** — bind a working directory once; chats and missions started inside a project inherit its cwd and stay grouped in their own sidebar section. Agents can create, rename, file into, and delete projects through the CLI too.
- **Mission controls** — stop, resume, or restart a single slot without restarting the mission; a restarted session comes back fresh with its original brief. Missions run in Bypass permission mode by default, with Accept-edits and Default a setting away, and never stall on an agent's first-run consent dialog.
- **Real terminals** — every pane is a real PTY on an `alacritty_terminal` grid drawn on the GPU: the agents' own colours, mouse reporting, IME input (Pinyin included), copy, file-path paste, 10,000 lines of scrollback. Click a file path to open it in your editor; select some output and ask about it in a side thread; ⌘+ and ⌘− zoom the app from 60% to 200%.
- **Terminal drawer** — every chat and every mission has a shell beneath it, one shortcut away, opened in the same directory as the agent above: run the tests the agent just wrote, check `git status`, tail a log, without leaving the pane or opening another terminal app. Drawers hold as many shells as you need and come back where you left them.

## Drive Runner from your agents

The bundled `runner` command is the one way agents, scripts, and people at a terminal drive the app. Its whole surface fits on a screen:

```sh
$ runner help
runner — operate Runner from a shell or a mission session

USAGE
  runner status
  runner project list|show|create|rename|delete
  runner role list|show|create|update|delete
  runner crew list|show|create|update|delete|add|set|remove|lead|order
  runner mission list|show|start|stop|resume|archive|unarchive|rename|pin|unpin|move|feed|answer
  runner chat start
  runner session list|show|stop|archive|resume|restart
  runner msg post|read
  runner signal <type>
  runner ask <question> | runner ask --human <prompt> --choices <a,b,...>
  runner call <tool> [<json>]

OUTPUT
  --json   print the JSON result
  -q       print only result ids

CONTEXT
  Inside a mission, msg post/read, signal, and ask use the event log directly.
  Outside, mission-scoped writes require --mission; --as names a roster handle.
```

On macOS, the first launch installs it to `~/.local/bin`, or a writable `/usr/local/bin`, when that directory is already on the login `PATH`; otherwise it is one click in **Settings → General → Command line**. On Windows, Runner adds its sidecar directory to the user `PATH`.

Agents need no setup. Runner installs a `runner` skill for every detected agent into two roots: `~/.claude/skills/` for Claude Code, and `~/.agents/skills/` for Codex, GitHub Copilot CLI, and pi. The skill points the agent at the version-matched `runner help agents` guide. The same **Command line** section has the `runner` command row and the **Runner skill for agents** switch.

A whole mission, driven from outside the app:

```sh
runner crew list --json
mission=$(runner mission start --crew <crew> --goal-file - -q < brief.md)
runner mission feed "$mission" --follow --json
runner mission show "$mission" --json
runner msg post --mission "$mission" --to <lead_handle> "message"
runner mission answer "$mission" <question_id> <choice>
runner mission stop "$mission"
runner mission archive "$mission"
```

Agents use `--json`; without it, list and show commands render tables and readable summaries for people. Exit status 0 is success, 1 means Runner refused the operation, 2 is a usage or reference error, 3 means the app is not running, and 5 means a sandbox blocked the local connection. Inside a mission, the same binary takes its mission and handle from the environment and is how crew members message and signal each other. The part that compounds: your daily agent can plan a fix, dispatch a coder and reviewer crew to build it, and keep working, while every session it spawned is still a real terminal you can open and watch.

## Supported agents

| | Claude Code | Codex | GitHub Copilot CLI | pi | Antigravity CLI |
| --- | :---: | :---: | :---: | :---: | :---: |
| Chats, missions, resume after relaunch | ✓ | ✓ | ✓ | ✓ | ✓ |
| Runs on Windows | ✓ | ✓ | ✓ ¹ | ✓ ² | off by default ³ |
| Fork a chat | ✓ | ✓ | — | ✓ | — |
| Working / Idle from the agent's hooks | ✓ | ✓ | ✓ | ✓ | macOS only |
| Needs you: approval and question dialogs shown | ✓ | — | ✓ | from extensions only | — |
| Model list read from the CLI | ✓ | ✓ | — | ✓ | — |
| Update from Settings → Agents | ✓ | ✓ | ✓ | ✓ | — |
| Permission modes | Default · Accept edits · Auto · Bypass | Default · Auto · Bypass | Default · Accept edits · Bypass | — | Default · Accept edits · Bypass |
| Skills pane | catalog + on/off | catalog + on/off | catalog + on/off | catalog | catalog |
| Runner skill installed | ✓ | ✓ | ✓ | ✓ | — |
| Terminal rendering covered by fixtures | ✓ | ✓ | — | — | — |

¹ GitHub Copilot CLI runs natively on Windows but has not been smoke-tested there yet.
² pi runs natively on Windows but has not been smoke-tested there yet; its bash tool requires Git for Windows.
³ Antigravity CLI stays off by default on Windows until it has been smoke-tested there; you can switch it on in **Settings → Agents**.

Claude Code and Codex are the primary agents, with tuned launch and nudge timing. GitHub Copilot CLI needs a Copilot subscription. pi brings your own configured model provider. Antigravity CLI signs in with a Google account and updates itself when it starts, so it has no **Update** button. [Issues](https://github.com/yicheng47/runner/issues) are welcome.

Install the agent CLIs separately. Runner detects them on `PATH`, with per-agent executable overrides in **Settings → Agents**, which also shows each CLI's version and, when a newer one is published, an **Update** button that runs the CLI's own updater in a terminal. On Windows, Claude Code and pi's bash tool require Git for Windows; npm-based CLI installations require Node.js. PowerShell 7 is optional. Agents run natively on Windows, without WSL.

<img src="assets/agents.png" alt="Settings → Agents — each detected agent CLI with its executable, model, effort, and an enable toggle" width="100%" />

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
- [`dev-crew/`](./examples/dev-crew/) — an architect / impl / reviewer trio: one decomposes, one builds, one audits
- [`docs-crew/`](./examples/docs-crew/) — architect partitions a complex repo, 2+ writers draft per-module docs in parallel, editor harmonizes
- [`tic-tac-toe/`](./examples/tic-tac-toe/) — 2 agents + 1 referee actually playing a game against each other
- [`werewolf/`](./examples/werewolf/) — 6-player social deduction with a god moderator
- [`tomb-raid/`](./examples/tomb-raid/) — a 4-person heist crew run by a DM

Each is a copy-pasteable handle + system-prompt set you can spawn into a new Crew and hit Start.

## Acknowledgements

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** and **[gpui-ce](https://github.com/gpui-ce/gpui-ce)** — the UI is built on gpui-ce, the community fork that keeps Zed's GPU-accelerated UI framework published and usable outside Zed. Thank you to the Zed team for building and open-sourcing the framework, and to the gpui-ce maintainers for carrying it forward; Zed's terminal crates were the architectural reference for Runner's terminal split.
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — the terminal grid, parser, and scrollback under every pane.
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — the procedural box-drawing glyph table is transcribed from the WebGL addon under its MIT notice (`crates/runner-app/LICENSE.xterm`).
- **[Windows Terminal ConPTY](https://github.com/microsoft/terminal)** — Windows builds bundle Microsoft's `conpty.dll` and `OpenConsole.exe` under the MIT notice (`crates/runner-app/LICENSE.conpty`).
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## Author

Runner is written and maintained by **Yicheng Wang** (Jason Wang, 王逸成) — [@yicheng47](https://github.com/yicheng47) on GitHub.

## License

MIT. Copyright (c) 2026 Yicheng Wang (Jason Wang). You can use, modify, and redistribute Runner for anything, including at work and inside closed-source products, as long as you keep the copyright notice (see `LICENSE`). Versions released from 2026-08-22 through 2026-09-22 were published under GPL-3.0-only and remain available under that license.
