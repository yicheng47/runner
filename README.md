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
  <a href="https://discord.gg/6BrfZdHKNf"><img src="https://img.shields.io/badge/Discord-join%20community-5865F2?style=flat-square&logo=discord&logoColor=white" alt="Join Runner on Discord" /></a>
  <a href="#community"><img src="https://img.shields.io/badge/WeChat-user%20group-07C160?style=flat-square&logo=wechat&logoColor=white" alt="WeChat user group" /></a>
  <a href="https://runnersh.dev/downloads/"><img src="https://img.shields.io/badge/macOS%20%7C%20Windows-native-2ea44f?style=flat-square" alt="macOS and Windows" /></a>
  <a href="https://ko-fi.com/yichengwang"><img src="https://img.shields.io/badge/Sponsor-Ko--fi-FF5E5B?style=flat-square" alt="Sponsor Runner on Ko-fi" /></a>
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
  <img src="assets/mission_feed.png" alt="Runner mission workspace — a shared mission feed with @coder handing off to @reviewer alongside live crew session cards" width="100%" />
</p>

<p align="center">
  <a href="#demo">Demo</a>
  ·
  <a href="#mobile">Mobile</a>
  ·
  <a href="#features">Features</a>
  ·
  <a href="#supported-agents">Agents</a>
  ·
  <a href="#runner-cli">CLI</a>
  ·
  <a href="#example-crew">Crew example</a>
  ·
  <a href="#community">Community</a>
  ·
  <a href="./CONTRIBUTING.md">Contributing</a>
</p>

> Alpha · Native macOS (Apple Silicon and Intel) and Windows (x64), with Linux (ARM64, x86_64) in beta.

Linux support is in beta from Runner 0.18.2, with signed `.deb` and tarball packages for ARM64 and x86_64 and in-app updates. It has been tried on an Ubuntu 26.04 ARM64 virtual machine; native hardware, x86_64 and other distributions are not yet verified. See [Linux build and install instructions](docs/arch/linux.md) and [validation status](docs/tests/archive/823-linux-support.md).

Runner brings CLI coding agents together in reusable crews, with real terminals and a shared mission feed. Your sessions and history stay on your machine.

## Demo

A tour from first launch: setup finds your agents and builds a starter crew, a mission runs in real terminals and asks you a question, you review the change in the side panel, then follow along from your phone.

https://github.com/user-attachments/assets/13e4d920-a803-47d5-8a99-8078b364bc3e

## Mobile

**Runner for iOS** is available via public TestFlight: [Join on TestFlight →](https://testflight.apple.com/join/CRdfZzmS)

Use your iPhone to follow live terminals, start chats and missions, and answer your crew on a paired computer. Remote control as a whole is still in private beta—the relay server is in private beta and currently available in mainland China only, while Tailscale support for users outside mainland China is under development. To join the private beta, [sign up for the waitlist on the official website →](https://runnersh.dev/beta/)

## Features

- **Reusable crews** — Compose saved roles into a crew, define shared team conventions, adjust per-slot agent and model overrides, and launch a mission with the same team whenever you need it.
- **Missions and coordination** — Each mission gives every crew slot a live terminal and a shared feed where agents coordinate and bring questions to you. Stop, resume or restart one slot without touching the rest. [Architecture →](./docs/arch/arch.md)
- **Missions report back to their chat** — A mission started from a chat links to it and groups under it in the sidebar, so the agent can end its turn instead of watching. When the mission needs you, fails or settles, Runner types one line into the chat and the agent reads what happened. [Linked chat and reports →](./docs/guide/concepts/missions.md#linked-chat-and-reports)
- **Remote control** — Switch to another computer in the sidebar or connect from your iPhone to use its live terminals, start chats and missions, and answer your crew with end-to-end encryption. Remote control is currently in private beta because the relay server is in private beta for mainland China only; Tailscale support for outside mainland China is in development. [Join the beta waitlist →](https://runnersh.dev/beta/) · [Remote access guide →](./docs/guide/guides/remote-access.md)
- **Real terminals** — Each agent keeps its own TUI in a real PTY on a GPU-drawn `alacritty_terminal` grid, with mouse reporting, IME input (Pinyin included), file-path paste and click-to-open, and a shell drawer beneath every mission.
- **Split tabs and windows** — Split a tab as far as the window allows, drag panes to reorder, group tabs into folders, and open more windows with `⇧⌘N` or `Ctrl+Shift+N`.
- **Files beside your agents** — Read the checkout an agent works in and its git status in the side panel without leaving Runner: highlighted code, rendered Markdown, a file finder on `⇧⌘O`, and `⌘`-click on terminal paths. [Files guide →](./docs/guide/guides/chats-and-windows.md#browse-files-in-the-side-panel)
- <a id="runner-cli"></a>**The `runner` CLI** — Your agents, scripts and terminal can start chats, launch crews and follow missions with the `runner` command, even with the app closed (`--json` supported). Runner installs a skill for detected agents so they can drive it too. [CLI guide →](./docs/guide/guides/drive-runner-from-agents.md)
- **Agents keep working when Runner is closed** — Sessions live in `runnerd`, Runner's background service. Quit the app, or let it crash, and reopen it to the same live terminals; stopped agents resume their own conversations on the next launch.
- **MCP servers and skills in one place** — **Settings → MCP** and **Settings → Skills** list each agent's own config and switch entries off per agent, changing only the entry you touched.
- **Desktop notifications** — Get notified when an agent needs input, finishes a turn or fails, and click to open its chat or mission. Choose events in **Settings → Notifications**.
- **Projects** — Bind a working directory once; chats and missions started in it inherit the cwd and group in the sidebar.
- **Light and dark** — Runner Light and Runner, plus Catppuccin, Rosé Pine, Tokyo Night, Gruvbox and Solarized, picking a theme for each mode that sets the app and terminal colours together. Claude Code follows the switch live.

## Support Runner

If Runner is useful to you, consider supporting its ongoing development and maintenance.

- [![Sponsor Runner on Ko-fi](https://img.shields.io/badge/Sponsor%20Runner-Ko--fi-FF5E5B?style=for-the-badge)](https://ko-fi.com/yichengwang) — Recommended for international supporters using PayPal.
- [![Sponsor Runner on Afdian](https://img.shields.io/badge/Sponsor%20Runner-Afdian-946BE7?style=for-the-badge)](https://afdian.com/a/runnersh) — Recommended for supporters in mainland China using WeChat Pay or Alipay.

## Community

- Join the [Runner Discord](https://discord.gg/6BrfZdHKNf) for questions, workflows, bug reports and contribution discussions.
- Bugs and feature requests: [GitHub Issues](https://github.com/yicheng47/runner/issues).
- Thanks to [LINUX DO](https://linux.do) for providing a community for discussion and sharing open-source projects.
- Scan the QR code below to join the Runner WeChat group for setup help, workflows and feedback. Updated on 2026-11-17; group QR codes expire after 7 days. If it has expired, [open an issue](https://github.com/yicheng47/runner/issues/new) so I can refresh it.

<img src="assets/wechat_group_qr.png" alt="Runner 微信用户群二维码" width="200" />

## Supported agents

Runner integrates with **Codex, Claude Code, Antigravity CLI, GitHub Copilot CLI, pi, Cursor, Grok Build and OpenCode v2**. **TRAE CLI** is available as an opt-in runtime. Features and platform support vary by agent.

[Agent setup, capabilities and platform notes →](./docs/guide/reference/supported-agents.md)

## Example crew

Fresh installations include **Pair coding crew**: a coder implements and a reviewer checks the diff until the review is clean. Both use Codex by default; you can choose a different agent for either role.

[Build a crew and explore example prompts →](./docs/guide/guides/build-a-crew.md)

## Acknowledgements

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** and **[gpui-pre](https://github.com/longbridge/gpui-kit)** — the UI uses a crates.io snapshot of Zed's GPU-accelerated UI framework. Thank you to the Zed team for building and open-sourcing GPUI, and to gpui-kit and Jason Lee (huacnlee) for publishing gpui-pre; Zed's terminal crates were the architectural reference for Runner's terminal split.
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — the terminal grid, parser, and scrollback under every pane.
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — the procedural box-drawing glyph table is transcribed from the WebGL addon under its MIT notice (`crates/runner-app/LICENSE.xterm`).
- **[Windows Terminal ConPTY](https://github.com/microsoft/terminal)** — Windows builds bundle Microsoft's `conpty.dll` and `OpenConsole.exe` under the MIT notice (`crates/runner-app/LICENSE.conpty`).
- **[Catppuccin](https://github.com/catppuccin/catppuccin)** (MIT), **[Rosé Pine](https://github.com/rose-pine/rose-pine-theme)** (MIT), **[Tokyo Night](https://github.com/folke/tokyonight.nvim)** (Apache-2.0), **[Gruvbox](https://github.com/morhetz/gruvbox)** (MIT) and **[Solarized](https://ethanschoonover.com/solarized/)** (MIT) — bundled app and terminal themes (`crates/runner-app/LICENSE.themes`).
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## Author

Runner is written and maintained by **Yicheng Wang** (Jason Wang, 王逸成) — [@yicheng47](https://github.com/yicheng47) on GitHub.

## License

MIT. Copyright (c) 2026 Yicheng Wang (Jason Wang). You can use, modify, and redistribute Runner for anything, including at work and inside closed-source products, as long as you keep the copyright notice (see `LICENSE`). Versions released from 2026-08-22 through 2026-09-22 were published under GPL-3.0-only and remain available under that license.
