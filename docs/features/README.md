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

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.
