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
- [37 — Agent runtime executable settings](./37-agent-runtime-executable-settings.md) — detect and display Claude Code/Codex executables from the user's login-shell environment, fix slow shell initialization failures, and provide explicit per-runtime path overrides.
- [45 — Auto-resume on launch](./45-auto-resume-on-launch.md) — stamp quit-killed running chats and mission-slot sessions with `resume_on_launch`, then auto-resume them (staggered, resume-only, settings-gated) on next open; crash path never stamps.
- [52 — Hook-based session status](./52-hook-based-session-status.md) — authoritative `working`/`waiting`/`done` status from agent CLI hooks injected per spawn (claude `--settings`, codex hooks.json — never the user's config), with the byte-flow IdleDetector demoted to a universal fallback tier; adds the needs-you attention state.
- [60 — Fork a chat into a split pane or a new tab](./60-fork-chat-to-pane-or-tab.md) — native-only session fork (claude-code `--resume <key> --fork-session`) with a destination choice: split pane beside the source in the current tab, or a new tab; supersedes 53's native tier after #348's revisit trigger fired ([#398](https://github.com/yicheng47/runner/issues/398)).
- [56 — Backend list pagination](./56-backend-list-pagination.md) — Runners/Crews pagination moves into SQL (`page`/`page_size`/`query` with LIMIT/OFFSET and the search filter server-side); the pager becomes one slim row flush under the cards, hidden at a single page, with no half-clipped card above it ([#377](https://github.com/yicheng47/runner/issues/377)).
- [57 — Start Project modal](./57-start-project-modal.md) — replace the bare directory-picker "Add project" flow with a modal mirroring the chat/mission start modals: directory field prefilled from `settings.defaultWorkingDir`, name field defaulting to the directory basename and following it until manually edited ([#383](https://github.com/yicheng47/runner/issues/383)).
- [58 — Runner & crew detail redesign](./58-runner-crew-detail-redesign.md) — Pencil-first redesign of both MVP-draft detail pages: crew detail puts the slot roster above the prose config sections instead of below them, runner detail clamps the system-prompt dump and consolidates redundant cards ([#393](https://github.com/yicheng47/runner/issues/393)).
- [59 — Per-slot model and effort overrides](./59-slot-model-effort-overrides.md) — complete the slot-as-agent-config model: model and effort overridable per slot without requiring a runtime override first (new `effort_override` column, resolver rework, ungated chips in the crew editor) ([#397](https://github.com/yicheng47/runner/issues/397)).

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
- [24 — Cronjobs](./24-cronjobs.md)
  — closed as won't-do ([#193](https://github.com/yicheng47/runner/issues/193)):
  a resident scheduler (overlap, catch-up, timeouts, wake correctness)
  is always-on machinery inside an app whose identity is a cockpit you
  open to work in, and `mission_start` over MCP/CLI already lets any
  external scheduler fire missions on cron with zero app code. Revisit
  only if the same mission goal keeps getting launched manually on a
  rhythm.
- [53 — Session fork](./53-session-fork.md)
  — closed as won't-do ([#348](https://github.com/yicheng47/runner/issues/348)):
  months of daily use produced zero fork reaches, and the generic
  transcript-handoff tier was exactly the lossy per-runtime capture
  machinery the simplicity budget keeps refusing. The revisit trigger
  fired in 2026-08; the native tier returns, with destinations, as
  spec 60 ([#398](https://github.com/yicheng47/runner/issues/398)).
