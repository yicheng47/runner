# GitHub Copilot CLI runtime — implementation plan

Plan for feature [540](../features/540-copilot-cli-runtime.md) ([#540](https://github.com/yicheng47/runner/issues/540)). The spec carries the per-surface inventory and the probe evidence; this file is the mission sequence and what has landed.

## Status (2026-09-16)

Phase 0 done: `cmp/MarkCopilot` (`QGeFI`) and the frame `Spec — GitHub Copilot CLI runtime (540) · v1` (`OBtYk`) on `design/runner.pen`; Jason chose GitHub's Copilot Purple `#8534F3` for the mark. Mission 1 (codex peer, `01M2M1Z7AMVEG6JJQ9Y6JJHY5T`) reviewed clean the same day and was checkpointed as `b1f5437` on `feat/540-copilot-runtime`; Jason's smoke pass is deferred, so the branch is not merged. Mission 1 merged as [PR #607](https://github.com/yicheng47/runner/pull/607) (`a308719`) after Jason moved Copilot to the third Agents row. Mission 2 reviewed clean the same day and Jason passed its smoke; it lands together with the Agents-row provider marks and the Skills pane fixes (Copilot caption, wider runtime picker, no "off" count without toggles).

## Missions

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| 1 | Spec phases 1 and 2: enum, adapter argv, trust preseed, defaults, catalog, MCP client, title filter, mark, permission copy, every enumerating test, README and arch docs. No hook adapter, no fixture. | codex peer, [brief](./archive/gpui-rewrite/briefs/540-m1-copilot-runtime.md) | Merged via [PR #607](https://github.com/yicheng47/runner/pull/607), `a308719`; Jason's smoke passed 2026-09-16 |
| 2 | Spec phase 3: the hook status adapter on macOS — Runner-owned `--plugin-dir` plugin, `copilot_status.rs`, `HookStatusWatcher::Copilot`, decision 3's event mapping — as a slice under `docs/impls/347-hook-status/`. | codex peer, [brief](./archive/gpui-rewrite/briefs/540-m2-copilot-hook-status.md) | Reviewed clean 2026-09-16 (`01M2MASYZ4ZSBEXGAC66Q5H3YB`); Jason's smoke passed; landed with the Agents marks and Skills pane fixes |
| 3 | Spec phase 4: smoke on macOS and JASONPC, the terminal fixture, archive the spec. | Jason + inline | Not started |

## Decisions that bind

The spec's decisions 1–9. In particular for mission 1: keys are caller-assigned with no capture thread (8); the trust seed runs before every spawn and touches only `trustedFolders` (2); chats assert no permission posture (4, [#596](../features/596-chat-permission-posture.md)); the static model list comes from the installed binary's `copilot help config` (5); the mark is `#8534F3` fixed in both themes (9).

## Log

- 2026-09-16 — spec revived from a live probe of 1.0.83 (hooks via `--plugin-dir`, trust dialog survives `--yolo`, `Notification(permission_prompt | elicitation_dialog)` observed), inventory written, design frame signed off, mission 1 briefed.
