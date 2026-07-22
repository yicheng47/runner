# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

## Index

- [05 — Cross-platform, agent-agnostic MCP & skills management](./05-runner-skills.md) —
  one central catalog of MCP servers and skills, stored in a neutral
  shape and materialized per agent (claude-code JSON, codex TOML,
  skill dirs) with a cross-platform apply mechanism; informed by the
  skills-manager reference analysis.
- [19 — Mission split view](./19-mission-split-view.md) — per-mission
  pane tree with drag-tab-to-edge splitting; render two PTYs (or
  feed + PTY) side-by-side inside one mission window; complements
  spec 12 (multi-window can't side-by-side same-mission PTYs).
- [21 — Import native agent sessions into a project](./21-import-native-sessions.md)
  — project-level import of existing claude-code / codex sessions
  whose recorded cwd matches the project directory; the follow-up
  importer that feature 40 deferred, superseding the old
  detect-and-resume Start Chat picker shape.
- [24 — Cronjobs](./24-cronjobs.md)
  — scheduled recurring missions dispatched to a crew on a cron
  expression; in-process Tokio scheduler, skip-on-overlap, one
  missed-tick catch-up; new sidebar section between MISSION and CHAT.
- [37 — Agent runtime executable settings](./37-agent-runtime-executable-settings.md) — detect and display Claude Code/Codex executables from the user's login-shell environment, fix slow shell initialization failures, and provide explicit per-runtime path overrides.
- [43 — Sidebar pinned section](./43-sidebar-pinned-section.md) — global PINNED view for tabs and missions, with persisted drag reorder, origin-scope exclusion, and append-on-unpin semantics over the node tree.
- [44 — Sidebar node tree](./44-sidebar-node-tree.md) — replace `folders`/`tabs` + `project_id` pointer grouping + pin flags with one `nodes` table (`parent_id` + `position` as the single containment/ordering mechanism); merges the MISSION/CHAT sections and unlocks reorder-anywhere, missions-in-containers, and unified drag.
- [45 — Auto-resume on launch](./45-auto-resume-on-launch.md) — stamp quit-killed running chats and mission-slot sessions with `resume_on_launch`, then auto-resume them (staggered, resume-only, settings-gated) on next open; crash path never stamps.
- [46 — Sidebar project reorder](./46-sidebar-project-reorder.md) — drag project rows to reorder them within the PROJECTS section; frontend-only wiring over the existing `node_move` root-scope support, with project drags presenting other project rows as positions instead of containers.
- [47 — Deferred mission nudge delivery](./47-deferred-mission-nudge-delivery.md) — park router stdin nudges while the recipient pane has pending local input and flush when it clears, so notifications stop submitting the user's half-typed drafts; pending-input tracking in the session manager plus a coalescing per-session outbox in the router.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
