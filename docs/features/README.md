# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

## Index

- [05 — Agent-agnostic MCP & skills management](./05-runner-skills.md) —
  one central catalog of MCP servers and skills, stored in a neutral
  shape and materialized per agent (claude-code JSON, codex TOML,
  skill dirs); informed by the skills-manager reference analysis.
- [24 — Cronjobs](./24-cronjobs.md)
  — scheduled recurring missions dispatched to a crew on a cron
  expression; in-process Tokio scheduler, skip-on-overlap, one
  missed-tick catch-up; new sidebar section between MISSION and CHAT.
- [37 — Agent runtime executable settings](./37-agent-runtime-executable-settings.md) — detect and display Claude Code/Codex executables from the user's login-shell environment, fix slow shell initialization failures, and provide explicit per-runtime path overrides.
- [45 — Auto-resume on launch](./45-auto-resume-on-launch.md) — stamp quit-killed running chats and mission-slot sessions with `resume_on_launch`, then auto-resume them (staggered, resume-only, settings-gated) on next open; crash path never stamps.
- [52 — Hook-based session status](./52-hook-based-session-status.md) — authoritative `working`/`waiting`/`done` status from agent CLI hooks injected per spawn (claude `--settings`, codex hooks.json — never the user's config), with the byte-flow IdleDetector demoted to a universal fallback tier; adds the needs-you attention state.
- [53 — Session fork](./53-session-fork.md) — branch a chat into a new session: native full-history fork for claude-code via `--resume <key> --fork-session`, bounded ring-transcript handoff draft for other runtimes; original session untouched.

## Archive

Shipped specs live in [`archive/`](./archive/), in spec-number order.
See the directory listing for what's there.

## Dropped

Considered and deliberately not built. Spec kept in-repo as the record.

- [19 — Mission split view](./19-mission-split-view.md)
  — closed as won't-do ([#255](https://github.com/yicheng47/runner/issues/255)):
  crew missions coordinate turn-based, so side-by-side slot PTYs mostly
  show one busy terminal next to an idle one; the feed + per-runner tabs
  cover monitoring, and split view already exists for direct chats.
- [21 — Import native agent sessions into a project](./21-import-native-sessions.md)
  — closed as won't-do ([#176](https://github.com/yicheng47/runner/issues/176)):
  the CLIs' own resume pickers (`claude --resume` / `codex resume` from a
  pane in the project cwd) cover the core need, so the native-store import
  machinery wasn't worth its maintenance surface.
