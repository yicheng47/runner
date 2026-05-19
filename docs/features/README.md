# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

## Index

- [01 — Archived tab](./01-archived-tab.md) — view and unarchive
  missions and chats that fell off the active sidebar.
- [03 — MCP server](./03-mcp-server.md) — expose Runner's CRUD surface
  over HTTP MCP so external Claude Code / Codex can configure the app.
- [05 — Skills + MCPs management per runner](./05-runner-skills.md) —
  attach reusable skills and MCP servers to runner templates; injected
  natively at spawn via a per-session synthetic agent home.
- [11 — Runner avatar](./11-runner-avatar.md) — procedural 5×5
  identicon ("bits graph") derived from the handle, rendered in the
  rail, sidebar, mission roster, and chat header.
- [12 — Multi-window frontend](./12-multi-window.md) — spawn
  additional Tauri windows for missions / chats; Arc-style overlay
  when two windows look at the same subject; PTY mounts only in the
  primary.
- [14 — Human notifications](./14-human-notifications.md) — macOS
  system notification when an agent posts to `@human` or fires
  `ask_human`; suppress when the relevant mission/chat is already
  in foreground.
- [15 — Light theme](./15-light-theme.md) — Solarized Light palette
  applied via CSS variable override on `<html data-theme>`; new
  `Auto · Light · Dark` setting; app icon stays brand-constant.
- [16 — Sidebar mission detail](./16-sidebar-mission-detail.md) —
  expand the selected mission row to show goal, cwd, crew, started
  time; inline edit for goal and cwd via folder picker.
- [17 — Sidebar folders](./17-sidebar-folders.md) — user-defined
  folders that group missions + chats together (Arc-style),
  replacing the forced MISSIONS / DIRECT CHATS top-level split;
  Inbox pseudo-folder for the un-organized.
- [19 — Mission split view](./19-mission-split-view.md) — per-mission
  pane tree with drag-tab-to-edge splitting; render two PTYs (or
  feed + PTY) side-by-side inside one mission window; complements
  spec 12 (multi-window can't side-by-side same-mission PTYs).
- [20 — Drop the per-crew signal allowlist](./20-drop-signal-allowlist.md)
  — remove `crews.signal_types`, the per-crew `signal_types.json`
  sidecar, and the CLI's file-backed validation. Replace with a
  code-side enum in `runner-core`. Pure cleanup; no product change.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
