# Mission Brief: #1429 (Slice 1) — Extract usage & accounts state from NativeRoot

## Starting point

- **Issue:** Part of [#1429](https://github.com/yicheng47/runner/issues/1429) (item 1 of [#1248](https://github.com/yicheng47/runner/issues/1248); slice 1 of 3 — Usage & accounts)
- **Branch:** `refactor/1429-native-root-usage`
- **Worktree:** `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1429-native-root-usage`
- **Base revision:** `a084538b` (`origin/main`)
- **Parallel isolation:** Active peer worktrees exist for `feat/1342-unified-markdown-renderer`, `fix/1395-remote-project-drag`, and `test/flaky-process-probe-tests`. Never touch those worktrees or assume their commits landed. Keep this worktree's own `target/` (never set shared `CARGO_TARGET_DIR`).

## Problem and outcome

`NativeRoot` (`crates/runner-app/src/surfaces/root/mod.rs`) still carries the 7-field Usage & accounts cluster directly as loose fields: `usage_open`, `usage_refresh_task`, `claude_account_picker`, `usage_anchor`, `usage_installed`, `usage_checked`, and `usage_enabled`.

Following the same mechanical pattern established by #1450 (`SidePanelState` / `SidebarPreviewState`), extract those 7 fields into a plain Rust struct `UsageState` in `crates/runner-app/src/surfaces/app_shell/usage.rs`, owned as a single `pub(crate) usage: UsageState` field on `NativeRoot` at the same declaration position.

Move pure state-only helpers that operate exclusively on `UsageState`'s fields onto `UsageState`, while keeping `cx.spawn`, `cx.notify()`, and cross-surface coordination on `NativeRoot`.

## Read first

- `AGENTS.md` (`Test Scope` mechanical-refactor rubric, `Commit And PR Conventions`, `Crew Missions`)
- `crates/runner-app/src/surfaces/root/mod.rs` (`NativeRoot` struct) and `crates/runner-app/src/surfaces/root/init.rs` and `store.rs`
- `crates/runner-app/src/surfaces/app_shell/usage.rs` and `crates/runner-app/src/surfaces/app_shell/sidebar.rs`
- `crates/runner-app/src/surfaces/settings/usage.rs` and `crates/runner-app/src/surfaces/settings/page/mod.rs`
- `crates/runner-app/src/surfaces/side_panel.rs` (reference pattern from #1450)

## Mechanical-refactor boundaries and non-goals

This mission is strictly a **mechanical refactor** under `AGENTS.md#test-scope` so no live QA is required. Every constraint below is a hard boundary:

- **Plain Rust struct only.** Do NOT introduce new GPUI `Entity` types, split GPUI views, or change `cx.notify()`, `cx.observe()`, subscriptions, `FocusHandle` wiring, element render/hit-test trees, or `usage_refresh_task` async/timer/drop lifetimes. Keep the 7 fields in their existing relative order inside `UsageState` and place `usage` at the same position on `NativeRoot` so drop order is preserved identically.
- **No behavior change.** Popover open/close, refresh cadence, Claude account picker creation/reset, installed/checked/enabled tracking, and remote/local switching must behave identically.
- **Preserve all test assertions.** Update field paths in existing tests (`app_shell/tests.rs`, `settings/page/tests.rs`, `remote_client/tests.rs`, etc.) to reach `root.usage.*` without deleting or weakening any assertion.
- **Out of scope:** `agent_updates`, the remaining #1429 clusters (`chat_lifecycle`, `panes_and_splits` — separate PRs), #1430, #1431, and any changes outside `crates/runner-app/`.

## Validation and handoffs

- **Coder (`@coder`):** Implement the mechanical extraction cleanly, verify with `make verify` (workspace check, tests, clippy, updater clippy, fmt-check), and hand off the commit/diff to `@reviewer`. Never run the desktop app.
- **Reviewer (`@reviewer`):** Review the diff in full against `AGENTS.md`'s mechanical-refactor rubric. Verify zero changes to GPUI `Entity`/notify/observe/subscription/focus/render/task-lifetime wiring, preserved drop order, zero behavior changes, and intact test assertions. If anything crosses a reactivity or ownership boundary, flag it as must-fix to keep the extraction purely mechanical. Confirm no live QA is needed under `AGENTS.md#test-scope`. Never run the desktop app.
- **PR handoff:** Once review is clean, `@coder` folds this brief into the implementation commit (no standalone brief commit), pushes `refactor/1429-native-root-usage`, opens a PR against `main` referencing `Part of #1429` (do NOT write `Closes #1429`, since two clusters remain) with labels `frontend-only`, `no-behavior-change`, and `risk-low` (no `needs-live-test`), drives CI green on all platforms, and has `@reviewer` add the `reviewed` label with a PR comment naming the reviewed head SHA.

## Authorization

Approved by Jason on 2026-11-24 for the `neon duo` crew (`coder` + `reviewer`, 2 slots). Authorized to commit, push, open a pull request against `main`, and drive CI green; stop at an open, reviewed, CI-green PR and do not merge.
