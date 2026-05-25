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
- [21 — Resume existing sessions for the cwd](./21-resume-cwd-sessions.md)
  — Start Chat modal surfaces recent claude-code / codex sessions
  whose recorded cwd matches the picked working dir; selecting one
  passes the runtime's resume flag to the spawned child so the user
  continues their prior conversation in Runner.
- [22 — Collapsed rail mission + chat switcher](./22-collapsed-rail-mission-chat-switcher.md)
  — pinned mission + chat slots on the 52px rail with a status dot
  per slot and an overflow popover for everything else; hybrid
  "at-a-glance + everything-on-click" so the collapsed rail can
  answer "what am I working on now" without expanding.
- [23 — Drag-to-reorder chats and missions](./23-drag-reorder-chats-missions.md)
  — manual reorder via drag-and-drop (and `Cmd+Shift+↑/↓`) in the
  sidebar's mission + chat lists; cross-pinned-boundary drag also
  pins/unpins; fractional `sort_index` so each drop is one UPDATE.
  Follow-up to spec 22's deferred "manual reorder".

- [24 — Cronjobs](./24-cronjobs.md)
  — scheduled recurring missions dispatched to a crew on a cron
  expression; in-process Tokio scheduler, skip-on-overlap, one
  missed-tick catch-up; new sidebar section between MISSION and CHAT.

- [25 — Direct chat runtime picker](./25-direct-chat-runtime-picker.md)
  — Start Chat modal gains a "Runtime" mode alongside the existing
  "Runner" mode: pick a runtime (claude-code, codex) + cwd and go,
  no runner template required. Nullable `runner_id` on sessions.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
