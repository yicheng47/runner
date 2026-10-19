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
  <img src="assets/hero.png" alt="Runner start screen — start a new chat, terminal or crew mission from one workspace" width="100%" />
</p>

<p align="center">
  <a href="#features">Features</a>
  ·
  <a href="#demo">Demo</a>
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
      <a href="assets/remote_control.png"><img src="assets/remote_control.png" alt="Runner's computer picker listing this Mac and another computer on the same account" width="100%" /></a>
    </td>
    <td width="50%" valign="middle">
      <h3>Remote control</h3>
      <p>Pick another computer in the sidebar to use its live terminals, start chats and missions, and answer your crew. Connections are encrypted end to end. <a href="./docs/guide/guides/remote-access.md">Remote access guide →</a></p>
      <p><strong>Private beta:</strong> invite-only, currently available in mainland China.</p>
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
      <pre><code>runner crew list
runner mission start \
  --crew "Pair coding crew" \
  --goal-file brief.md
runner mission feed &lt;mission-id&gt; --follow
runner help agents</code></pre>
    </td>
    <td width="50%" valign="middle">
      <a name="runner-cli"></a>
      <h3>The <code>runner</code> CLI</h3>
      <p>Your agents, scripts and terminal can start chats, launch crews and follow missions with the <code>runner</code> command, even with the app closed. Add <code>--json</code> for machine-readable output. Runner installs a skill for detected agents so they can drive it too. <a href="./docs/guide/guides/drive-runner-from-agents.md">CLI guide →</a></p>
    </td>
  </tr>
</table>

**Also included:**

- **Agents keep working when Runner is closed** — Sessions live in `runnerd`, Runner's background service. Quit the app, or let it crash, and reopen it to the same live terminals; stopped agents resume their own conversations on the next launch.
- **MCP servers and skills in one place** — **Settings → MCP** and **Settings → Skills** list each agent's own config and switch entries off per agent, changing only the entry you touched.
- **Desktop notifications** — Get notified when an agent needs input, finishes a turn or fails, and click to open its chat or mission. Choose events in **Settings → Notifications**.
- **Projects** — Bind a working directory once; chats and missions started in it inherit the cwd and group in the sidebar.
- **Light and dark** — Carbon and Runner Light, plus Catppuccin, with an app palette and a terminal palette per mode. Claude Code follows the switch live.

## Demo

A quick tour of reusable roles, crew setup, split terminals, and coder/reviewer/QA coordination in a live mission.

https://github.com/user-attachments/assets/c2209015-1497-403a-98bd-cdc5502bee47

## Support Runner

If Runner is useful to you, consider supporting its ongoing development and maintenance.

- [![Sponsor Runner on Ko-fi](https://img.shields.io/badge/Sponsor%20Runner-Ko--fi-FF5E5B?style=for-the-badge)](https://ko-fi.com/yichengwang) — Recommended for international supporters using PayPal.
- [![Sponsor Runner on Afdian](https://img.shields.io/badge/Sponsor%20Runner-Afdian-946BE7?style=for-the-badge)](https://afdian.com/a/runnersh) — Recommended for supporters in mainland China using WeChat Pay or Alipay.

## Community

- Join the [Runner Discord](https://discord.gg/6BrfZdHKNf) for questions, workflows, bug reports and contribution discussions.
- Bugs and feature requests: [GitHub Issues](https://github.com/yicheng47/runner/issues).
- Thanks to [LINUX DO](https://linux.do) for providing a community for discussion and sharing open-source projects.
- 中文用户可以扫码加入 Runner 微信用户群。群二维码 7 天过期，如果扫码提示失效，请[提一个 issue](https://github.com/yicheng47/runner/issues/new) 提醒我更新。

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
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## Author

Runner is written and maintained by **Yicheng Wang** (Jason Wang, 王逸成) — [@yicheng47](https://github.com/yicheng47) on GitHub.

## License

MIT. Copyright (c) 2026 Yicheng Wang (Jason Wang). You can use, modify, and redistribute Runner for anything, including at work and inside closed-source products, as long as you keep the copyright notice (see `LICENSE`). Versions released from 2026-08-22 through 2026-09-22 were published under GPL-3.0-only and remain available under that license.
