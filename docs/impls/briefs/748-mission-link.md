# #748 — Link a chat to the missions it starts

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-748-linked-chat-migration`, on `feat/748-linked-chat-migration`, the branch of draft PR [#1129](https://github.com/yicheng47/runner/pull/1129). Its first commit, `feat(db): store a mission's linked chat and its report flag` (migration 0029), is on the remote as `05c16fe9`. This worktree rebased it onto `origin/main` at `3ecbf70c` without pushing, and the tip is this brief. Use this worktree's own `target/`. The root checkout stays on `main`.

## Problem and outcome

[#748](https://github.com/yicheng47/runner/issues/748): a chat that starts a mission hears nothing back unless its agent polls, and the sidebar shows no relation between the two. The outcome is the whole feature in the spec: a mission links to the direct agent chat that started it or that the person links it to by hand, Runner types a one-line `[runner]` report into that chat when the mission needs the person, fails or settles, and the sidebar and Active view show the chat and its missions as one linked group.

The spec, [docs/features/748-mission-link.md](../../features/748-mission-link.md), is signed off and wins on behaviour; this brief only adds what the spec leaves to the mission. The design is `runner/desktop/specs/748-coordinator-missions.pen` in `~/repos/yeelinks/yeelinks_design` (commit `301aaf0`), exported as PNGs in this worktree's ignored `target/design-ref/`: `sidebar-linked-group-E.png`, `active-agents-H.png`, `linking-by-hand.png` and `reports-and-back-link.png`. They win on visual detail; match them with the existing theme tokens, the sample data is illustrative. Do not delete them, and do not edit `.pen` files.

## Read first

The spec in full. `docs/arch/arch.md` §8.5 (inbox delivery), §9.3–§9.6 and §10.1. Then: `runtimes/helpers.rs::hook_env` and `cli_install::install_session_runner_shim` (identity); `ops/mission.rs` (`mission_start`, and #1130's answer validation, which just landed); `router/dispatch.rs` and `SessionManager::delivery_readiness` in `session/manager/` (the attempt that reports reuse, including #1126's confirmed-notice fix); `crates/runner-app/src/notifications.rs` (today's state derivation); `runner-core/src/protocol/mission.rs` and the companion protocol's mission types; `surfaces/sidebar/` (`active.rs`, `drag.rs`, `menus.rs`, `rows.rs`) and `surfaces/sidebar_logic.rs` (drop targets); `runner-core/src/protocol/agent_skill.rs` and `runner-cli/src/help.rs`.

## Deliverables

Phases 1–4 of the spec's Implementation phases, in that order, each as its own commit on top of the migration commit, each building and passing its tests: backend link, mission state and reports, app, then CLI, skill and docs. Fold the brief's commit, which also updates the spec's crew line and guide list, into the backend-link commit. The migration commit stays first; fixes to it fold into it.

Where the spec leaves a detail open, use these defaults and say so in the PR:

- **A drop on a tab holding several agent chats** opens the Link to chat submenu limited to that tab's chats.
- **The mission state** reaches the app and remote clients as an additive field beside `linked_chat` on mission summaries and details, so the app stops deriving it itself.
- **An Ended mission** (stopped, or every session exited) stays in its group showing Ended until it is archived.
- **#990's filters** are not on `main`. Skip the spec's filter-on-a-group test; whichever of #748 and #990 lands second adapts the other.
- **Guide pages** change in English and in `docs/guide/zh-CN/` together, per `docs/guide-authoring.md`; run `python3 script/check-guide-translations.py` and `make guide-cli`.

Non-goals are the spec's Out of scope list.

## Validation

In-process: every test the spec lists under each phase and under Verification → In-process, with the affected crates' tests on locked dependencies, `cargo fmt --all --check`, workspace Clippy with warnings denied (`--profile ci`, and `--features updater` on macOS), and `git diff --check`. Drive CI green on every platform.

Live (qa): the spec's Verification → Live, macOS checklist on the frozen candidate in the development app under Test Scope, with Codex and Claude Code chats on inexpensive models. On its first turn qa confirms the checklist's prerequisites, including a Claude Code sign-in in the development app. Record each check's result and evidence in `docs/tests/748-mission-link.md`; a check that cannot run is Blocked, never passed.

## Review and handoffs

Review goes phase by phase instead of one working-tree diff at the end. The first review also covers the migration commit, which no reviewer has seen yet. After a phase's clean review, coder commits it locally and moves to the next. qa receives one candidate after phase 4's clean review and runs the whole checklist on it.

## Authorization

Jason approved this three-agent claude trio mission on 2026-11-03. It may commit, push this branch, force-pushing over the pre-rebase `05c16fe9` with `--force-with-lease`, and finish in PR #1129 instead of opening a new PR. Exceptions to the crew's conventions: coder may push each reviewed phase to the draft so CI runs early, and the PR stays a draft until qa's verdict is in and CI is green. Then coder retitles #1129 for the whole feature, rewrites its body with the QA verdict, keeps `core-review` and `risk-high`, and marks it ready for review. The PR closes #748 with a closing keyword only if every live check passed.
