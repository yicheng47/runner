# GitHub Copilot CLI runtime — implementation plan

Plan for feature [540](../features/540-copilot-cli-runtime.md) ([#540](https://github.com/yicheng47/runner/issues/540)). The spec carries the per-surface inventory and the probe evidence; this file is the mission sequence and what has landed.

## Status (2026-09-16)

Phase 0 done: `cmp/MarkCopilot` (`QGeFI`) and the frame `Spec — GitHub Copilot CLI runtime (540) · v1` (`OBtYk`) on `design/runner.pen`; Jason chose GitHub's Copilot Purple `#8534F3` for the mark. Mission 1 briefed and started on the codex peer crew on branch `feat/540-copilot-runtime`.

## Missions

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| 1 | Spec phases 1 and 2: enum, adapter argv, trust preseed, defaults, catalog, MCP client, title filter, mark, permission copy, every enumerating test, README and arch docs. No hook adapter, no fixture. | codex peer, [brief](./archive/gpui-rewrite/briefs/540-m1-copilot-runtime.md) | Started 2026-09-16 |
| 2 | Spec phase 3: the hook status adapter on macOS — Runner-owned `--plugin-dir` plugin, `copilot_status.rs`, `HookStatusWatcher::Copilot`, decision 3's event mapping — as a slice under `docs/impls/347-hook-status/`. | tbd | Not started |
| 3 | Spec phase 4: smoke on macOS and JASONPC, the terminal fixture, archive the spec. | Jason + inline | Not started |

## Decisions that bind

The spec's decisions 1–9. In particular for mission 1: keys are caller-assigned with no capture thread (8); the trust seed runs before every spawn and touches only `trustedFolders` (2); chats assert no permission posture (4, [#596](../features/596-chat-permission-posture.md)); the static model list comes from the installed binary's `copilot help config` (5); the mark is `#8534F3` fixed in both themes (9).

## Log

- 2026-09-16 — spec revived from a live probe of 1.0.83 (hooks via `--plugin-dir`, trust dialog survives `--yolo`, `Notification(permission_prompt | elicitation_dialog)` observed), inventory written, design frame signed off, mission 1 briefed.
