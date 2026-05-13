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
