# Feature specs

Active feature specs, one per open tracking issue. A spec moves to [`archive/`](./archive/) once its issue closes, shipped or dropped: the implementation is the source of truth and the archived spec is the "what we were going for" record. Priorities and milestones live on the issues; [`../roadmap.md`](../roadmap.md) mirrors them.

Since 2026-09-01 a spec's number **is** its tracking issue number: file the issue first, then name the spec after it, so gaps in the sequence belong to bugs and PRs. Specs 01–64 predate the alignment and keep their numbers in `archive/`; the then-active ones were renumbered to their issues (05→73, 58→393, 61→403), and 60 (fork a chat) kept its number and shipped 2026-09-01.

## Active

- [748 — Mission updates reach the chat that started the mission](./748-mission-watch-delivery.md) — Runner types a one-line notice into the starting chat for messages to the person, broadcasts, questions, failures and the end, through the slot delivery gate, so Codex chats watch missions with no host facility ([#748](https://github.com/yicheng47/runner/issues/748), P1, 0.12; spec under review).
- [777 — Runtime adapter trait](./777-runtime-adapter.md) — each agent runtime becomes one module behind a `RuntimeAdapter` trait, registered in one `match`, so adding a runtime stops touching dozens of files; a refactor with byte-identical behavior, landing before #723 and #764 ([#777](https://github.com/yicheng47/runner/issues/777), P1, 0.12; spec settled, lands as three PRs).
- [562 — Missions as containers](./562-mission-spawn.md) — the mission owns its roster, a mission starts from a crew or a role, and the lead or an outside seat spawns, lists, waits on and stops slots ([#562](https://github.com/yicheng47/runner/issues/562), P1, 0.13 headline with 704; plan under review).
- [704 — Send a prompt from one session to another](./704-session-send.md) — `runner session send` and `session wait`, the terminal layer beside 562's missions: types into a running chat or terminal when it is idle, with a reply line, and stays off the bus ([#704](https://github.com/yicheng47/runner/issues/704), P1, 0.13 release blocker and headline with 562; spec under review).
- [772 — Start chats, terminals and missions from the keyboard](./772-keyboard-create.md) — ⌘↵ starts from anywhere in Start a chat and Start mission, Direct | Role on one tab stop with ⌘1/⌘2, and ⌘T and ⇧⌘M beside ⌘N rather than one merged form ([#772](https://github.com/yicheng47/runner/issues/772), P2, 0.12; mission on `feat/772-keyboard-create`).
- [764 — Support MCP management for Pi](./764-pi-mcp-management.md) — proposed catalog, editing, multi-client sync and conflict detection for pi's user MCP servers; verify native config support and reconcile Runner registration with the shipped CLI-only coordination contract ([#764](https://github.com/yicheng47/runner/issues/764), P2, 0.13; spec under review).
- [559 — Command palette on ⌘⇧P](./559-command-palette.md) — the ⌘K overlay in command mode over the keymap ([#559](https://github.com/yicheng47/runner/issues/559), P2, unscheduled).
- [565 — i18n, 简体中文 first](./565-i18n.md) — a language setting, a compile-time catalog, a live switch ([#565](https://github.com/yicheng47/runner/issues/565), P2, 0.14).
- [586 — Shell status: process detection first](./586-shell-status-detection.md) — foreground-process detection for shell panes before semantic shell integration ([#586](https://github.com/yicheng47/runner/issues/586), P2, 0.14).
- [701 — Desktop notifications](./701-desktop-notifications.md) — a popup Runner draws itself, following Zed's agent notification, when a session off screen waits on you, finishes or fails ([#701](https://github.com/yicheng47/runner/issues/701), P2, 0.13).

## Recently shipped

Reconciled on 2026-10-01 against closed issues and published releases. These specs are archived; their bodies preserve the original scope and dated design decisions.

| Spec | Shipped in |
| --- | --- |
| [706 — Plan usage for Claude Code and Codex](./archive/706-agent-usage.md) | 0.11.4, 2026-09-23 |
| [533 — Update agent CLIs from Settings → Agents](./archive/533-agent-cli-updates.md) | 0.11.5, 2026-09-26 |
| [575 — Splits follow the shell's live cwd](./archive/575-live-cwd.md) | 0.11.5, 2026-09-26 |
| [393 — Role page redesign](./archive/393-role-page.md) | 0.12.0, 2026-09-27 |
| [699 — Crew page redesign](./archive/699-crew-page.md) | 0.12.0, 2026-09-27 |
| [740 — Codex speed in role settings](./archive/740-codex-role-speed.md) | 0.12.1, 2026-09-28 |
| [724 — Preserve terminal wheel coordinates](./archive/724-terminal-wheel-coordinates.md) | 0.12.2, 2026-09-28 |
| [644 — Antigravity CLI runtime](./archive/644-antigravity-runtime.md) | 0.12.3, 2026-09-29; native Windows smoke pending |
| [735 — Start a chat modal redesign](./archive/735-start-chat-modal.md) | 0.12.5, 2026-09-30 |
| [756 — Model and effort in the chat side panel](./archive/756-chat-side-panel-model-effort.md) | 0.12.5, 2026-09-30 |
| [731 — Role side panel redesign](./archive/731-role-side-panel.md) | `main` ([PR #770](https://github.com/yicheng47/runner/pull/770)), 2026-10-01; next 0.12.x |
| [768 — New role and New crew open their pages](./archive/768-new-role-crew-pages.md) | `main` ([PR #770](https://github.com/yicheng47/runner/pull/770)), 2026-10-01; next 0.12.x |

## Dropped

Considered and deliberately not built; the spec stays in `archive/` as the record.

- [19 — Mission split view](./archive/19-mission-split-view.md) ([#255](https://github.com/yicheng47/runner/issues/255)): missions coordinate turn-based, so side-by-side slot PTYs mostly show one busy terminal next to an idle one.
- [21 — Import native agent sessions](./archive/21-import-native-sessions.md) ([#176](https://github.com/yicheng47/runner/issues/176)): the CLIs' own resume pickers from a pane cover the need.
- [24 — Cronjobs](./archive/24-cronjobs.md) ([#193](https://github.com/yicheng47/runner/issues/193)): a resident scheduler is always-on machinery inside a cockpit; `mission start` from an external scheduler covers it.
- [53 — Session fork](./archive/53-session-fork.md) ([#348](https://github.com/yicheng47/runner/issues/348)): zero reaches in months of use; the native tier returned as spec 60 ([#398](https://github.com/yicheng47/runner/issues/398)).
- [466 — Sessions outlive the app](./archive/466-sessions-outlive-the-app.md) ([#466](https://github.com/yicheng47/runner/issues/466), 2026-09-02): PTYs stay in-process; the direction returned as the session host ([#645](https://github.com/yicheng47/runner/issues/645), 0.14).
- [491 — Confirm quit while work is running](./archive/491-confirm-quit-running-work.md) ([#491](https://github.com/yicheng47/runner/issues/491), 2026-09-07): a stopgap the session host makes obsolete.
- [511 — Ask about a selection](./archive/511-ask-about-selection.md) ([#511](https://github.com/yicheng47/runner/issues/511), 2026-09-10): no side thread forked from a selection.
- [557 — Translucent window backdrop](./archive/557-window-backdrop.md) ([#557](https://github.com/yicheng47/runner/issues/557), 2026-09-11): not important enough to carry.
- [403 — Opt-in worktree isolation per mission](./archive/403-mission-worktree-isolation.md) ([#403](https://github.com/yicheng47/runner/issues/403), 2026-09-22): Runner is not an agent development environment; a crew that needs its own checkout makes one from its brief.
- [468 — Landing page](./archive/468-landing-page.md) ([#468](https://github.com/yicheng47/runner/issues/468), 2026-09-22): visitors land on the README, which already explains the model; a short mission demo covers the rest.
