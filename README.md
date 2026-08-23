<!-- LOGO -->
<h1 align="center">
  <img src="assets/icon.png" alt="Runner" width="128" />
  <br />
  Runner
</h1>

<p align="center">
  Spawn a runner. Create your crew. Ship the feature.
  <br />
  A local agentic development environment (ADE) — orchestrate crews of CLI coding agents: Claude Code, Codex, and friends.
</p>

<p align="center">
  <a href="#about">About</a>
  ·
  <a href="#features">Features</a>
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

---

> Status: alpha, actively shipping. macOS, Apple Silicon and Intel (one universal build).

---

## About

Runner is a local desktop workspace for operating multiple CLI coding agents at once. Instead of scattering Claude Code and Codex sessions across terminal windows, you run them as an organized fleet — configured runners, composed crews, coordinated missions — from a single app.

Runner is an **agentic development environment (ADE)**. Where an IDE organizes buffers and a debugger around the code you write, an ADE organizes terminals, crews, and event feeds around the agents writing it. The operator's job shifts accordingly: assign roles, start missions, monitor progress, review diffs, and make the calls agents escalate to you.

The coordination model is explicit. A **runner** is a reusable agent configuration — runtime, role, system prompt, working directory. A **crew** composes runners with exactly one lead. Starting a **mission** spawns one real PTY per slot into a tabbed workspace where the crew coordinates over an append-only event log: handoffs and status flow between agents, and when a decision needs a human, `ask_human` surfaces it in the feed. Everything runs and persists locally — sessions are real processes on your machine, and the log is on-disk and replayable.

Runner also runs as an **MCP server**: any MCP client — including the agents themselves — can create crews, start missions, and steer them programmatically. See [Drive it from your agents](#drive-it-from-your-agents-mcp).

Runner is a native macOS app written in Rust: [gpui-ce](https://github.com/gpui-ce/gpui-ce) — the community-maintained fork of [Zed](https://zed.dev)'s GPUI — for the UI, `alacritty_terminal` for the terminal grid, SQLite for state. No webview.

## Download

Latest macOS build (universal `.dmg`, Apple Silicon and Intel) on the [releases page](https://github.com/yicheng47/runner/releases/latest). The rolling [`nightly`](https://github.com/yicheng47/runner/releases/tag/nightly) prerelease tracks this branch and updates itself through Sparkle. Signed and notarized; Linux and Windows are not supported.

<!-- TODO(demo): add a "## Demo" section here once the new hero video is recorded — a Peer
     Coding Crew mission on a real repo (mission start from a project → feed + per-slot
     terminals → coder/reviewer handoffs via the Runner CLI → ask_human surfacing → done). -->

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

Every pane is a real PTY behind an `alacritty_terminal` grid, drawn by GPUI on the GPU — claude-code, codex, and any modern TUI render with their actual ANSI palette, mouse reporting, alt-screen redraws, and pixel-snapped box-drawing glyphs. Mouse selection and ⌘C, IME composition (Pinyin included), file-path paste, 10,000 lines of scrollback. Sessions are resumable across app restarts; the event log is the source of truth.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/multi_window.png" alt="Two Runner windows working different chats side by side" width="100%" />
</td>
<td width="50%" valign="middle">

### Multi-window

`⇧⌘N` opens additional OS windows — a mission on one screen, a wall of chats on the other. Windows coordinate ownership of shared sessions: the primary owns the PTY, and any other window showing the same session gets a hand-off overlay instead of a corrupted terminal.

</td>
</tr>
<tr>
<td width="50%">
  <img src="assets/mcp_settings.png" alt="Settings → MCP — one-click config for Claude Code and Codex" width="100%" />
</td>
<td width="50%" valign="middle">

### Drive it from your agents (MCP)

Everything above is also an MCP tool. Runner bundles a `runner-mcp` stdio sidecar, and **Settings → MCP** registers it with Claude Code, Codex, Qoder, or TRAE in one click. Connected agents assemble crews, start and steer missions (`mission_start`, `mission_feed`, `mission_post_human_signal`), and spin up chats (`session_start_direct`). The compounding trick: your daily driver agent plans a fix, dispatches a coder/reviewer crew, and keeps working — agents dispatching crews of agents, every session still a real PTY you can open and watch.

</td>
</tr>
</table>

### Also in the box

- **Projects** — bind a working directory once; chats and missions started inside a project inherit its cwd and stay grouped in their own sidebar section.
- **Themes** — Auto / Light / Dark chrome with two variants per side (Runner and Catppuccin Mocha dark; Codex Light and Catppuccin Latte light), independent terminal palettes (Runner, Catppuccin Mocha, Solarized Dark), Inter bundled as the UI font, and MesloLGS Nerd Font bundled for terminals.
- **Auto-update** — Sparkle, with a hint on the sidebar's Settings row when a new build is waiting; nightly and release feeds are signed with the same key.
- **Bundled `runner` CLI** — spawned agents message each other, check the crew roster, and post signals from inside their own PTYs.
- **Runtimes** — Claude Code and Codex are first-class: daily-driven, with fixture-tested terminal rendering and tuned launch/nudge timing. Qoder and TRAE run through the same paths but see far less use and may have rough edges — [issues](https://github.com/yicheng47/runner/issues) are welcome. All four are detected on `PATH`, with per-runtime executable overrides in **Settings → Agents**.

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

For dev setup, prereqs, and contributor conventions see [AGENTS.md](./AGENTS.md).

## Acknowledgements

- **[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)** and **[gpui-ce](https://github.com/gpui-ce/gpui-ce)** — the UI is built on gpui-ce, the community fork that keeps Zed's GPU-accelerated UI framework published and usable outside Zed. Thank you to the Zed team for building and open-sourcing the framework, and to the gpui-ce maintainers for carrying it forward; Zed's terminal crates were the architectural reference for Runner's terminal split.
- **[alacritty_terminal](https://github.com/alacritty/alacritty)** — the terminal grid, parser, and scrollback under every pane.
- **[xterm.js](https://github.com/xtermjs/xterm.js)** — the procedural box-drawing glyph table is transcribed from the WebGL addon under its MIT notice (`crates/runner-app/LICENSE.xterm`).
- **[Sparkle](https://sparkle-project.org)** — the macOS updater.

## License

GPL-3.0-only. Copyright (C) 2026 Jason Wang. Runner is free software: you can use it for anything, including at work, and redistribute or modify it under the terms of the GNU General Public License v3.0 — modified versions you distribute must stay under the same license (see `LICENSE`). Versions released before 2026-08-22 were published under MIT and remain so.
