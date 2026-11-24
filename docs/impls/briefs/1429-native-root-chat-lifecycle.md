# Mission Brief: #1429 (Slice 2) — Extract chat lifecycle state from NativeRoot

## Starting point

- **Issue:** Part of [#1429](https://github.com/yicheng47/runner/issues/1429) (item 1 of [#1248](https://github.com/yicheng47/runner/issues/1248); slice 2 of 3 — Chat lifecycle)
- **Branch:** `refactor/1429-native-root-chat-lifecycle`
- **Worktree:** `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1429-native-root-chat-lifecycle`
- **Base revision:** `64f81420` (`origin/main`)
- **Parallel isolation:** Active peer worktrees exist for `feat/1342-unified-markdown-renderer`, `fix/1395-remote-project-drag`, and `test/flaky-process-probe-tests`. Never touch those worktrees or assume their commits landed. Keep this worktree's own `target/` (never set shared `CARGO_TARGET_DIR`).

## Problem and outcome

`NativeRoot` (`crates/runner-app/src/surfaces/root/mod.rs`) carries the 6-field Chat lifecycle cluster directly as loose fields: `stopping_sessions`, `chat_transitions`, `next_chat_transition_generation`, `session_exit_codes`, `fork_confirm`, and `forking_sessions` (along with the `ChatTransition` and `ForkConfirm` types defined in `surfaces/root/mod.rs`).

Following the same mechanical pattern established by #1450 (`SidePanelState` / `SidebarPreviewState`) and #1457 (`UsageState`), extract those 6 fields (and `ChatTransition` / `ForkConfirm` if cleanly owned by `surfaces/chat`) into a plain Rust struct `ChatLifecycleState` in `crates/runner-app/src/surfaces/chat/mod.rs`, owned as a single `pub(crate) chat_lifecycle: surfaces::chat::ChatLifecycleState` field on `NativeRoot`.

Move pure state-only helpers that operate exclusively on those fields (transition generation/insertion/clearing, fork confirmation and tracking, stopping/exit-code bookkeeping) onto `ChatLifecycleState`, while keeping `cx.spawn`, `cx.notify()`, daemon RPCs, focus routing, and cross-surface coordination on `NativeRoot`.

## Read first

- `AGENTS.md` (`Test Scope` mechanical-refactor rubric, `Commit And PR Conventions`, `Crew Missions`)
- `crates/runner-app/src/surfaces/root/mod.rs` (`NativeRoot`, `ChatTransition`, `ForkConfirm`) and `crates/runner-app/src/surfaces/root/init.rs`
- `crates/runner-app/src/surfaces/chat/mod.rs`, `attachment.rs`, `close.rs`, and `actions.rs`
- `crates/runner-app/src/surfaces/app_shell/usage.rs` (reference pattern from #1457)

## Mechanical-refactor boundaries and non-goals

This mission is strictly a **mechanical refactor** under `AGENTS.md#test-scope` so no live QA is required. Every constraint below is a hard boundary:

- **Plain Rust struct only.** Do NOT introduce new GPUI `Entity` types, split GPUI views, or change `cx.notify()`, `cx.observe()`, subscriptions, `FocusHandle` wiring, element render/hit-test trees, or async/timer lifetimes.
- **No behavior change.** Chat stop/resume/fork transitions, fork confirmation gating, generation cancellation, exit-code recording, and focus-on-completion behavior must remain identical.
- **Preserve all test assertions.** Update field paths in existing tests (`chat/mod.rs`, `chat/close_tests.rs`, `panes/tests.rs`, `remote_client/tests.rs`, `front_page/tests.rs`, `start_chat/launcher/tests.rs`, etc.) to reach `root.chat_lifecycle.*` without deleting or weakening any assertion.
- **Out of scope:** The Panes & splits cluster (`#1429` slice 3 — separate PR), #1430, #1431, and any changes outside `crates/runner-app/`.

## Validation and handoffs

- **Coder (`@coder`):** Implement the mechanical extraction cleanly, verify with `make verify` (workspace check, tests, clippy, updater clippy, fmt-check), and hand off the commit/diff to `@reviewer`. Never run the desktop app.
- **Reviewer (`@reviewer`):** Review the diff in full against `AGENTS.md`'s mechanical-refactor rubric. Verify zero changes to GPUI `Entity`/notify/observe/subscription/focus/render/timer wiring, zero behavior changes, and intact test assertions. If anything crosses a reactivity or ownership boundary, flag it as must-fix to keep the extraction purely mechanical. Confirm no live QA is needed under `AGENTS.md#test-scope`. Never run the desktop app.
- **PR handoff:** Once review is clean, `@coder` folds this brief into the implementation commit (no standalone brief commit), pushes `refactor/1429-native-root-chat-lifecycle`, opens a PR against `main` referencing `Part of #1429` (do NOT write `Closes #1429`, since slice 3 Panes & splits remains) with labels `frontend-only`, `no-behavior-change`, and `risk-low` (no `needs-live-test`), drives CI green on all platforms, and has `@reviewer` add the `reviewed` label with a PR comment naming the reviewed head SHA.

## Authorization

Approved by Jason on 2026-11-24 for the `neon duo` crew (`coder` + `reviewer`, 2 slots). Authorized to commit, push, open a pull request against `main`, and drive CI green; stop at an open, reviewed, CI-green PR and do not merge.
