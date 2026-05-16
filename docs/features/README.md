# Feature specs

In-progress and planned feature specs. Specs for shipped features are
deleted — the implementation is the source of truth.

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

## Index

- [01 — Archived tab](./01-archived-tab.md) — view and unarchive
  missions and chats that fell off the active sidebar.
- [02 — Collapsable sidebar](./02-collapsable-sidebar.md) — toggle
  between the full sidebar and a narrow icon rail.
- [03 — MCP server](./03-mcp-server.md) — expose Runner's CRUD surface
  over HTTP MCP so external Claude Code / Codex can configure the app.
- [04 — New-messages pill](./04-new-messages-pill.md) — floating pill in
  the workspace feed that tells the user new events arrived while they
  were scrolled up; click to jump to bottom.
- [05 — Skills + MCPs management per runner](./05-runner-skills.md) —
  attach reusable skills and MCP servers to runner templates; injected
  natively at spawn via a per-session synthetic agent home.
- [08 — Hide system signals from the mission feed](./08-hide-system-signals-from-feed.md)
  — drop router-internal `inbox_read` / `runner_status` /
  `mission_warning` rows from the workspace feed; NDJSON log remains
  the audit trail.
- [09 — Persistent auto-update toast](./09-auto-update-toast.md) —
  keep the update toast visible through `downloading` and `ready`, not
  just the millisecond-long `available` window; surface `Restart`
  directly so background auto-installs stop happening in secret.
- [10 — Mission session persistence](./10-mission-session-persistence.md) —
  stop killing alive mission panes on app restart; mount the mission
  bus + router eagerly so direct-chat-style persistence works for
  missions too.
- [11 — Runner avatar](./11-runner-avatar.md) — procedural 5×5
  identicon ("bits graph") derived from the handle, rendered in the
  rail, sidebar, mission roster, and chat header.
- [12 — Multi-window frontend](./12-multi-window.md) — spawn
  additional Tauri windows for missions / chats; Arc-style overlay
  when two windows look at the same subject; PTY mounts only in the
  primary.
- [13 — PTY-silence idle detection](./13-pty-silence-idle-detection.md)
  — derive runner busy/idle from forwarder-observed PTY output
  silence instead of relying on the agent to call `runner status`;
  works for any TUI without runner-CLI integration.
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
- [18 — App logging + crash reporting](./18-app-logging-and-crash-reporting.md)
  — `tauri-plugin-log` writing to `~/Library/Logs/<bundle>/` +
  panic hook that captures backtraces + Help → Reveal logs in
  Finder; foundational so future user-reported crashes are
  recoverable.
