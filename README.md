<!-- LOGO -->
<h1 align="center">
  <img src="assets/icon.png" alt="Runner" width="128" />
  <br />
  Runner
</h1>

<p align="center">
  <a href="https://github.com/yicheng47/runner/stargazers"><img src="https://img.shields.io/github/stars/yicheng47/runner?style=flat-square&logo=github&label=stars" alt="GitHub stars" /></a>
  <a href="https://github.com/yicheng47/runner/releases"><img src="https://img.shields.io/github/downloads/yicheng47/runner/total?style=flat-square&label=downloads" alt="Downloads" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/yicheng47/runner?style=flat-square" alt="License" /></a>
  <a href="#community"><img src="https://img.shields.io/badge/WeChat-user%20group-07C160?style=flat-square&logo=wechat&logoColor=white" alt="WeChat user group" /></a>
  <a href="#download"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS and Windows" /></a>
</p>

<p align="center">
  English · <a href="./README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <strong>Spawn a runner. Create your crew. Ship the feature.</strong>
  <br />
  A native terminal that orchestrates coding agents. Claude Code and Codex keep their own TUI; Runner adds sessions, skills, and crews.
</p>

<p align="center">
  <a href="https://github.com/yicheng47/runner/releases/latest"><strong>Download Runner</strong></a>
</p>

<p align="center">
  <img src="assets/hero.png" alt="Runner — one tab with a Claude Code and two Codex sessions side by side, projects and chats in the sidebar" width="100%" />
</p>

<p align="center">
  <a href="#about">About</a>
  ·
  <a href="#features">Features</a>
  ·
  <a href="#supported-agents">Agents</a>
  ·
  <a href="#drive-it-from-your-agents-mcp">MCP</a>
  ·
  <a href="#example-crew">Crew example</a>
  ·
  <a href="#download">Download</a>
  ·
  <a href="#documentation">Documentation</a>
  ·
  <a href="./AGENTS.md">Contributing</a>
</p>

> Status: alpha, actively shipping. Native macOS (Apple Silicon) and Windows (x64), with Windows support starting in 0.8.0.

## About

Runner is a local desktop workspace for operating multiple CLI coding agents at once. Instead of scattering Claude Code and Codex sessions across terminal windows, you run them as an organized fleet — configured runners, composed crews, coordinated missions — from a single app.

Runner is a **native terminal that orchestrates coding agents**. Each agent keeps its own TUI in a real PTY; Runner adds the session layer around it. Where an IDE organizes buffers and a debugger around the code you write, Runner organizes terminals, crews, and event feeds around the agents writing it. The operator's job shifts accordingly: assign roles, start missions, monitor progress, review diffs, and make the calls agents escalate to you.

The coordination model is explicit. A **runner** is a reusable agent configuration — runtime, role, system prompt, working directory. A **crew** composes runners with exactly one lead. Starting a **mission** spawns one real PTY per slot into a tabbed workspace where the crew coordinates over an append-only event log: handoffs and status flow between agents, and when a decision needs a human, `ask_human` surfaces it in the feed. Everything runs and persists locally — sessions are real processes on your machine, and the log is on-disk and replayable.

Runner also runs as an **MCP server**: any MCP client — including the agents themselves — can create crews, start missions, and steer them programmatically. See [Drive it from your agents](#drive-it-from-your-agents-mcp).

Runner is a native macOS and Windows app written in Rust: [gpui-ce](https://github.com/gpui-ce/gpui-ce) — the community-maintained fork of [Zed](https://zed.dev)'s GPUI — for the UI, `alacritty_terminal` for the terminal grid, SQLite for state. No webview.

## Download

Grab the latest build from the [releases page](https://github.com/yicheng47/runner/releases/latest): a signed and notarized `.dmg` for macOS on Apple Silicon, and a signed `Runner-Setup-…-x64.exe` installer for Windows 10 version 1809 or later. Intel Macs, Windows ARM64, and Linux are not supported.

Both platforms update in place, macOS through Sparkle and Windows through the update icon beside Settings, and keep your settings, chats, and missions. On Windows, SmartScreen may still warn on a fresh release while the certificate builds reputation; **More info → Run anyway** continues.

Want what landed today instead? The [`nightly` prerelease](https://github.com/yicheng47/runner/releases/tag/nightly) is built from `main` for both platforms, signed the same way, and updates on its own channel.

## Demo

A three-agent crew — two players and a referee — playing a game of tic-tac-toe against each other over the mission feed, from the [`tic-tac-toe`](./examples/tic-tac-toe/) example crew.

https://github.com/user-attachments/assets/fb3669a4-010d-42d0-9555-2a3ba3223c75

Also on [YouTube](https://www.youtube.com/watch?v=eKXcfxC4m1U) if the player above does not load.

## Features

<table>
<tr>
<td width="50%">
  <img src="assets/crew.png" alt="Crew editor — slots, prompts, team conventions, one lead" width="100%" />
</td>
<td width="50%" valign="middle">

### Crews — roles, prompts, one lead

A **runner** is a reusable agent configuration: runtime, role, system prompt, working directory. A **crew** composes runners into named slots with exactly one lead, plus team conventions and a definition of done that every mission inherits.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/mission_feed.png" alt="Mission workspace — the event feed between crew and human" width="100%" />
</td>
<td width="50%" valign="middle">

### Missions — a crew working one goal

Starting a mission spawns one live PTY per slot into a tabbed workspace where the crew coordinates over an append-only event log — every signal is persisted and replayable, so missions survive a quit or crash, and `ask_human` questions surface in the feed.

[Architecture →](./docs/arch/arch.md)

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/chat_split.png" alt="Chat tab with three split panes and organized sidebar" width="100%" />
</td>
<td width="50%" valign="middle">

### Chats — tabs, split panes, folders

Every chat is a real 1:1 PTY with a runner, no mission required. Tabs hold up to three side-by-side panes — run a Claude Code and a Codex on the same problem in one view. The sidebar groups tabs into collapsible folders; every tab shows a spinner while a pane is still working and a dot when one finished while you were elsewhere, so a wall of parallel agents stays scannable.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/mission_terminal.png" alt="Per-slot PTY terminal, live" width="100%" />
</td>
<td width="50%" valign="middle">

### A real terminal

Every pane is a real PTY behind an `alacritty_terminal` grid, drawn by GPUI on the GPU — claude-code, codex, and any modern TUI render with their actual ANSI palette, mouse reporting, alt-screen redraws, and pixel-snapped box-drawing glyphs. Mouse selection and copy (⌘C on macOS, Ctrl+C on Windows), IME composition (Pinyin included), file-path paste, 10,000 lines of scrollback. Sessions are resumable across app restarts; the event log is the source of truth.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/multi_window.png" alt="Two Runner windows working different chats side by side" width="100%" />
</td>
<td width="50%" valign="middle">

### Multi-window

`⇧⌘N` on macOS or `Ctrl+Shift+N` on Windows opens additional OS windows — a mission on one screen, a wall of chats on the other. Windows coordinate ownership of shared sessions: the primary owns the PTY, and any other window showing the same session gets a hand-off overlay instead of a corrupted terminal.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/mcp_settings.png" alt="Settings → MCP — every MCP server each agent has, with Runner's own pinned first" width="100%" />
</td>
<td width="50%" valign="middle">

### Drive it from your agents (MCP)

Everything above is also an MCP tool. Runner bundles a `runner-mcp` stdio sidecar and registers it with Claude Code, Codex and TRAE CLI from **Settings → Agents**. Connected agents assemble crews, create and file projects, start and steer missions (`mission_start`, `mission_feed`, `mission_post_human_signal`), and spin up chats (`session_start_direct`). The compounding trick: your daily driver agent plans a fix, dispatches a coder/reviewer crew, and keeps working — agents dispatching crews of agents, every session still a real PTY you can open and watch.

**Settings → MCP** is the other direction: one catalog of every MCP server each agent has configured, read from the agent's own config file, with a toggle per server and Runner's own pinned first.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/skills.png" alt="Settings → Skills — every skill an agent can load, with a toggle per skill" width="100%" />
</td>
<td width="50%" valign="middle">

### Skills — one list per agent

**Settings → Skills** lists the skills each agent loads, read from the agent's own skill directory. Click a row to read a skill, hover it to edit, flip the toggle to hide it from every new session of that agent, inside Runner or not. Runner writes only the agent's own override key, never the skill itself.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/appearance.png" alt="Settings → Appearance — light and dark previews with an app and a terminal palette per mode" width="100%" />
</td>
<td width="50%" valign="middle">

### Light, dark, and the terminal in between

Runner Light and Carbon are designed for Runner; Catppuccin Latte and Mocha ride along. **Settings → Appearance** picks an app palette and a terminal palette per mode above a live preview of both, so a light app gets a light terminal without a second setting, and a Claude Code chat follows the flip the moment it happens. Rosé Pine Dawn is the default light terminal.

</td>
</tr>
</table>

### Also in the box

- **Projects** — bind a working directory once; chats and missions started inside a project inherit its cwd and stay grouped in their own sidebar section. Agents can create, rename, file into, and delete projects over MCP too.
- **Terminal drawer** — a shell beneath every chat and every mission, in the same directory as the agent above it, one shortcut away.
- **Mission controls** — stop, resume, or restart a single slot without restarting the mission; a restarted runner comes back fresh with its original brief. Missions run in Bypass permission mode by default, with Accept-edits and Default a setting away, and never stall on an agent's first-run consent dialog.
- **Sessions that outlive the app** — quitting or crashing does not kill your agents; the next launch reattaches to the sessions still running, and a quit while work is in flight asks first.
- **Terminal extras** — click a file path in any terminal to open it in your editor; select some output and ask about it in a side thread forked from the chat; ⌘+ and ⌘− zoom the whole app from 60% to 200%.
- **Bundled `runner` CLI** — spawned agents message each other, check the crew roster, and post signals from inside their own PTYs.

## Supported agents

| Agent | macOS (Apple Silicon) | Windows (x64) |
| --- | --- | --- |
| Claude Code | Supported | Supported |
| Codex | Supported | Supported |
| TRAE CLI | Experimental | Not validated |

Claude Code and Codex are the primary supported agents, with fixture-tested terminal rendering and tuned launch/nudge timing. TRAE CLI sees less use and may have rough edges; it is enabled by default on macOS when detected, and disabled by default on Windows, where Runner integration has not been validated. [Issues](https://github.com/yicheng47/runner/issues) are welcome.

Install the agent CLIs separately. Runner detects them on `PATH`, with per-agent executable overrides in **Settings → Agents**. On Windows, Claude Code also requires Git for Windows for Git Bash; npm-based CLI installations require Node.js. PowerShell 7 is optional. Agents run natively on Windows, without WSL.

## Example crew

The **default Runner shape** is a two-runner peer-coding loop: one implements, one reviews, and the loop runs on the working-tree diff until the review is clean — no architect, no dispatch overhead, just the tightest loop that still has a second pair of eyes. Runner seeds this crew on first launch; the source lives in [`examples/peer-coding/`](./examples/peer-coding/).

| Runner | Runtime | Role | System prompt |
| --- | --- | --- | --- |
| **@coder** (lead) | `codex` | Branches, implements, runs the checks, hands the diff to the reviewer, fixes findings. | [`coder.md`](./examples/peer-coding/coder.md) |
| **@reviewer** | `codex` | Reads the working-tree diff, reports must-fix issues with file:line pointers, never edits code. | [`reviewer.md`](./examples/peer-coding/reviewer.md) |

The crew's team conventions — feature branch first, review before any commit, nothing merged unless the human asks — are in [`team-conventions.md`](./examples/peer-coding/team-conventions.md). Both slots ship on `codex`; switching `@coder` to `claude-code` makes it a cross-vendor pair, where each model catches what the other's training glosses over.

### More crews

For weirder, more fun crew shapes, peek at [`examples/`](./examples/):

- [`peer-coding/`](./examples/peer-coding/) — the default coder / reviewer pair above
- [`dev-crew/`](./examples/dev-crew/) — an architect / impl / reviewer trio: one decomposes, one builds, one audits
- [`docs-crew/`](./examples/docs-crew/) — architect partitions a complex repo, 2+ writers draft per-module docs in parallel, editor harmonizes
- [`tic-tac-toe/`](./examples/tic-tac-toe/) — 2 agents + 1 referee actually playing a game against each other
- [`werewolf/`](./examples/werewolf/) — 6-player social deduction with a god moderator
- [`tomb-raid/`](./examples/tomb-raid/) — a 4-person heist crew run by a DM

Each is a copy-pasteable handle + system-prompt set you can spawn into a new Crew and hit Start.

## Documentation

Architecture, runtime contracts, product vision, and per-feature specs live in [`docs/`](./docs/) — start with [`docs/arch/arch.md`](./docs/arch/arch.md) for the wire-level overview, or [`docs/product/vision.md`](./docs/product/vision.md) for the product direction.

macOS and Windows are developed together on `main`. For dev setup, prereqs, and contributor conventions see [AGENTS.md](./AGENTS.md).

## Acknowledgements

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** and **[gpui-ce](https://github.com/gpui-ce/gpui-ce)** — the UI is built on gpui-ce, the community fork that keeps Zed's GPU-accelerated UI framework published and usable outside Zed. Thank you to the Zed team for building and open-sourcing the framework, and to the gpui-ce maintainers for carrying it forward; Zed's terminal crates were the architectural reference for Runner's terminal split.
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — the terminal grid, parser, and scrollback under every pane.
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — the procedural box-drawing glyph table is transcribed from the WebGL addon under its MIT notice (`crates/runner-app/LICENSE.xterm`).
- **[Windows Terminal ConPTY](https://github.com/microsoft/terminal)** — Windows builds bundle Microsoft's `conpty.dll` and `OpenConsole.exe` under the MIT notice (`crates/runner-app/LICENSE.conpty`).
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## Author

Runner is written and maintained by **Yicheng Wang** (Jason Wang, 王逸成) — [@yicheng47](https://github.com/yicheng47) on GitHub.

## Community

- Bugs and feature requests: [GitHub Issues](https://github.com/yicheng47/runner/issues).
- 中文用户可以扫码加入 Runner 微信用户群。群二维码 7 天过期，如果扫码提示失效，请[提一个 issue](https://github.com/yicheng47/runner/issues/new) 提醒我更新。

<img src="assets/wechat_group_qr.png" alt="Runner 微信用户群二维码" width="200" />

## License

GPL-3.0-only. Copyright (C) 2026 Yicheng Wang (Jason Wang). Runner is free software: you can use it for anything, including at work, and redistribute or modify it under the terms of the GNU General Public License v3.0 — modified versions you distribute must stay under the same license (see `LICENSE`). Versions released before 2026-08-22 were published under MIT and remain so.
