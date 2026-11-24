# Mission Brief: #1429 (Slice 3) — Extract panes & splits state from NativeRoot

## Starting point

- **Issue:** Part of [#1429](https://github.com/yicheng47/runner/issues/1429) (item 1 of [#1248](https://github.com/yicheng47/runner/issues/1248); slice 3 of 3 — Panes & splits)
- **Branch:** `refactor/1429-native-root-panes-splits`
- **Worktree:** `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1429-native-root-panes-splits`
- **Base revision:** `e1609f28` (`origin/main`)
- **Parallel isolation:** Active peer branches/worktrees exist for `feat/1444-code-block-header`, `feat/1452-terminal-file-link-detection`, `fix/files-choose-another-folder-label`, `fix/1395-remote-project-drag`, and `test/flaky-process-probe-tests`. Never touch other worktrees or assume their commits landed. Keep this worktree's own `target/` (never set shared `CARGO_TARGET_DIR`).

## Problem and outcome

`NativeRoot` (`crates/runner-app/src/surfaces/root/mod.rs`) still carries the 15-field Panes & splits cluster directly as loose fields: `pane_session_key_copies`, `pane_worktree_copies`, `pane_action_menus`, `split_menus`, `pane_bounds`, `maximized_panes`, `pane_drop`, `pane_rename`, `_pane_rename_focus_subscription`, `_pane_close_focus_subscription`, `terminal_close_confirm`, `pending_pane_closes`, `pending_tab_close`, `split_sizes_dirty`, and `resizing_split` (along with the supporting types `PaneRename`, `TerminalCloseTarget`, `TerminalCloseConfirm`, `PendingPaneClose`, `PendingTabClose`, and `ArchiveAllSource` in `surfaces/root/mod.rs`).

Following the exact mechanical pattern established by #1450 (`SidePanelState` / `SidebarPreviewState`), #1457 (`UsageState`), and #1462 (`ChatLifecycleState`), extract those 15 fields (and their supporting pane/close types where cleanly owned by `surfaces/panes`) into a plain Rust struct `PaneState` under `crates/runner-app/src/surfaces/panes/` (e.g. `surfaces/panes/mod.rs` or `surfaces/panes/state.rs`), owned as a single `pub(crate) panes: surfaces::panes::PaneState` field on `NativeRoot` at the same declaration position.

Move pure state-only helpers that operate exclusively on `PaneState`'s fields (such as clearing/pruning per-pane maps, maximize lookups/toggles, split resize flag/state transitions, pending close bookkeeping) onto `PaneState`, while keeping `cx.spawn`, `cx.notify()`, focus routing, subscriptions setup, daemon RPCs, and cross-surface coordination on `NativeRoot`.

## Read first

- `AGENTS.md` (`Test Scope` mechanical-refactor rubric, `Commit And PR Conventions`, `Crew Missions`)
- `crates/runner-app/src/surfaces/root/mod.rs` (`NativeRoot` and supporting pane/close types) and `crates/runner-app/src/surfaces/root/init.rs`
- `crates/runner-app/src/surfaces/panes/{mod,pane,header,tab,tree,dialogs}.rs`
- `crates/runner-app/src/surfaces/chat/{close,layout,actions,mod}.rs`
- `crates/runner-app/src/surfaces/chat/mod.rs` (`ChatLifecycleState`, reference pattern from #1462)

## Mechanical-refactor boundaries and non-goals

This mission is strictly a **mechanical refactor** under `AGENTS.md#test-scope` so no live QA is required. Every constraint below is a hard boundary:

- **Plain Rust struct only.** Do NOT introduce new GPUI `Entity` types, split GPUI views, or change `cx.notify()`, `cx.observe()`, subscription setup/teardown, `FocusHandle` creation or focus restoration (`TerminalCloseConfirm::open`/`restore_focus`), element render/hit-test trees, or async/timer lifetimes. Preserve the exact relative order of `pane_rename`, `_pane_rename_focus_subscription`, `_pane_close_focus_subscription`, and `terminal_close_confirm` inside `PaneState` and keep `panes` at the same position on `NativeRoot` so drop order is identical.
- **No behavior change.** Pane menus, copy buttons, bounds tracking, split dragging/resizing, maximize/restore, pane rename, drag-drop, and close/archive confirmation flows must behave identically.
- **Preserve all test assertions.** Update field and type paths across existing tests (`panes/tests.rs`, `chat/close_tests.rs`, `remote_client/tests.rs`, `sidebar/link_tests.rs`, `front_page/tests.rs`, `settings/page/localization_tests.rs`, `mission_workspace/remote_tests.rs`, etc.) to reach `root.panes.*` without deleting or weakening any assertion.
- **Out of scope:** Unrelated `NativeRoot` fields (`archived_title_menu`, `archive_check_gates`, `tab_landed_at`, `drawer_resizing`, `sidebar_resizing`), large `impl NativeRoot` method moves outside this cluster (`start_chat/*`, `settings/remote.rs`), #1430, #1431, and any changes outside `crates/runner-app/`.

## Validation and handoffs

- **Coder (`@coder`):** Implement the mechanical extraction cleanly, verify with `make verify` (workspace check, tests, clippy, updater clippy, fmt-check), and hand off the commit/diff to `@reviewer`. Never run the desktop app.
- **Reviewer (`@reviewer`):** Review the diff in full against `AGENTS.md`'s mechanical-refactor rubric. Verify zero changes to GPUI `Entity`/notify/observe/subscription/focus/render/timer wiring, preserved drop order for the focus subscriptions and entities, zero behavior changes, and intact test assertions. If anything crosses a reactivity or ownership boundary, flag it as must-fix to keep the extraction purely mechanical. Confirm no live QA is needed under `AGENTS.md#test-scope`. Never run the desktop app.
- **PR handoff:** Once review is clean, `@coder` folds this brief into the implementation commit (no standalone brief commit), pushes `refactor/1429-native-root-panes-splits`, opens a PR against `main` referencing `Part of #1429` (do NOT write `Closes #1429`) with labels `frontend-only`, `no-behavior-change`, and `risk-low` (no `needs-live-test`), drives CI green on all platforms, and has `@reviewer` add the `reviewed` label with a PR comment naming the reviewed head SHA.

## Authorization

Approved by Jason on 2026-11-24 for the `neon duo` crew (`coder` + `reviewer`, 2 slots). Authorized to commit, push, open a pull request against `main`, and drive CI green; stop at an open, reviewed, CI-green PR and do not merge.
