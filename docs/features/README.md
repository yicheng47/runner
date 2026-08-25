# Feature specs

In-progress and planned feature specs. Shipped specs move to
[`archive/`](./archive/) once their tracking issue closes — the
implementation is the source of truth, but the spec stays around as the
"what we were going for" record (mirrors `docs/impls/archive/`).

Tracking lives in GitHub Issues with the `feature` label. Each spec
links to its tracking issue.

Since the GPUI rewrite shipped as `v0.6.0` (2026-08-23) there is one line of work on `main`; new features are Pencil-first in `design/runner.pen` and land as nightlies. The post-GA consolidation queue (M6) is [`../impls/gpui-rewrite/m6-remainder.md`](../impls/gpui-rewrite/m6-remainder.md), tracked in [#445](https://github.com/yicheng47/runner/issues/445).

## Index

- [05 — Agent-agnostic MCP & skills management](./05-runner-skills.md) —
  one central catalog of MCP servers and skills, stored in a neutral
  shape and materialized per agent (claude-code JSON, codex TOML,
  skill dirs); informed by the skills-manager reference analysis.
- [52 — Hook-based session status](./52-hook-based-session-status.md) — authoritative `working`/`waiting`/`done` status from agent CLI hooks injected per spawn (claude `--settings`, codex hooks.json — never the user's config), with the byte-flow IdleDetector demoted to a universal fallback tier; adds the needs-you attention state.
- [58 — Runner & crew detail redesign](./58-runner-crew-detail-redesign.md) — Pencil-first redesign of both MVP-draft detail pages: crew detail puts the slot roster above the prose config sections instead of below them, runner detail clamps the system-prompt dump and consolidates redundant cards ([#393](https://github.com/yicheng47/runner/issues/393)).
- [60 — Fork a chat into a split pane or a new tab](./60-fork-chat-to-pane-or-tab.md) — native-only session fork (claude-code `--resume <key> --fork-session`) with a destination choice: split pane beside the source in the current tab, or a new tab; supersedes 53's native tier after #348's revisit trigger fired ([#398](https://github.com/yicheng47/runner/issues/398)).
- [61 — Opt-in worktree isolation per mission](./61-mission-worktree-isolation.md) — mission-start decorator: opted-in missions get `git worktree add <repo>/.worktrees/… -b mission/<short-id>-<slug>` and `mission.cwd` points into it, so concurrent crews and the human's checkout never collide; default stays the project checkout, all slots share the tree, archive offers safe (`git worktree remove`, dirty-refusing) cleanup ([#403](https://github.com/yicheng47/runner/issues/403)).
- [64 — Native terminal as a pane option](./64-native-terminal.md) — `$SHELL` in a real PTY as pane content beside a chat in a split; a direct session with `runtime == "shell"`, no new `NodeType`, no modal, no archive; replaces the six-item pane header with a 26 px identity line (`⋯` for Stop/Rename/Archive, `×` to close the pane) and folds in per-pane rename; terminals respawn at their cwd on relaunch while chats stay behind resume-on-launch; the multi-tab design is deferred, not dropped ([#356](https://github.com/yicheng47/runner/issues/356)).

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
