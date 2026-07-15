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
- [05 — Cross-platform, agent-agnostic MCP & skills management](./05-runner-skills.md) —
  one central catalog of MCP servers and skills, stored in a neutral
  shape and materialized per agent (claude-code JSON, codex TOML,
  skill dirs) with a cross-platform apply mechanism; informed by the
  skills-manager reference analysis.
- [12 — Multi-window frontend](./12-multi-window.md) — spawn
  additional Tauri windows for missions / chats; Arc-style overlay
  when two windows look at the same subject; PTY mounts only in the
  primary.
- [19 — Mission split view](./19-mission-split-view.md) — per-mission
  pane tree with drag-tab-to-edge splitting; render two PTYs (or
  feed + PTY) side-by-side inside one mission window; complements
  spec 12 (multi-window can't side-by-side same-mission PTYs).
- [21 — Resume existing sessions for the cwd](./21-resume-cwd-sessions.md)
  — Start Chat modal surfaces recent claude-code / codex sessions
  whose recorded cwd matches the picked working dir; selecting one
  passes the runtime's resume flag to the spawned child so the user
  continues their prior conversation in Runner.
- [24 — Cronjobs](./24-cronjobs.md)
  — scheduled recurring missions dispatched to a crew on a cron
  expression; in-process Tokio scheduler, skip-on-overlap, one
  missed-tick catch-up; new sidebar section between MISSION and CHAT.
- [29 — Runner and crew list pagination and search](./29-runner-crew-list-pagination-search.md)
  — add shared search filters and client-side pagination to the
  Runners and Crews list pages so large template/team inventories
  stay scannable.
- [30 — OpenCode runtime](./30-opencode-runtime.md)
  — add OpenCode as a first-class runtime for direct chats, runner
  templates, and missions with conservative model/prompt support
  before permission/resume mappings.
- [33 — Mission last terminal tab](./33-mission-last-terminal-tab.md)
  — remember the last selected runner terminal inside each mission
  and restore it on workspace remount when the session is still valid.
- [36 — Keyboard shortcut rebinding](./36-keyboard-shortcut-rebinding.md) — customizable keybindings on the Settings shortcuts pane (#257 v2): recording state, unbind/restore, conflict detection, and handler indirection through the keymap registry.
- [37 — Agent runtime executable settings](./37-agent-runtime-executable-settings.md) — detect and display Claude Code/Codex executables from the user's login-shell environment, fix slow shell initialization failures, and provide explicit per-runtime path overrides.
- [38 — Sidebar folders for tabs](./38-sidebar-folders-for-tabs.md) — invert the sidebar hierarchy to Folder → Tab: collapsible user folders group chat tabs, every tab (single- or multi-pane) is one row, panes leave the sidebar; folders + tabs persist in SQLite, replacing the localStorage layout store.
- [39 — Chat working and unread-completion indicators](./39-chat-working-unread-indicators.md) — replace the removed sidebar lifecycle dot with a trailing tab spinner while any pane is working and a durable unread dot when the tab settles outside the focused visible chat; collapsed folders and CHAT roll hidden state upward.
- [40 — Projects](./40-projects.md) — cwd-bound project containers grouping chats and missions in a Codex-style PROJECT sidebar section; a `projects` table plus `project_id` on sessions and missions, direct folder-picker creation, and cwd inheritance for work started inside a project.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
