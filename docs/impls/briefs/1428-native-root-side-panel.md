# Mission Brief: #1428 — Extract side-panel and sidebar-preview state from NativeRoot

## Starting point

- **Issue:** [#1428](https://github.com/yicheng47/runner/issues/1428) (part of [#1248](https://github.com/yicheng47/runner/issues/1248), item 1, first slice)
- **Branch:** `refactor/1428-native-root-side-panel`
- **Worktree:** `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1428-native-root-side-panel`
- **Base revision:** `4a958228` (`origin/main`)
- **Parallel isolation:** Active peer worktrees exist for `feat/1342-unified-markdown-renderer` (PR #1449), `fix/1395-remote-project-drag`, and `test/flaky-process-probe-tests`. Never touch those worktrees, never assume their commits landed, and leave `surfaces/files/markdown.rs` untouched. Keep this worktree's own `target/` (never set shared `CARGO_TARGET_DIR`).

## Problem and outcome

`NativeRoot` (`crates/runner-app/src/surfaces/root/mod.rs`) holds two self-contained surface state clusters directly as loose fields: the right side-panel cluster (16 fields) and the collapsed-sidebar hover preview cluster (5 fields).

Extract each cluster into a plain Rust state struct owned directly by `NativeRoot` (one field per cluster on `NativeRoot`), and move methods that only operate on that cluster's fields onto the new struct:

1. **Side-panel state** (under `crates/runner-app/src/surfaces/side_panel.rs` or a submodule of `surfaces/side_panel/`): move `chat_panel_resizing`, `chat_panel_visibility`, `chat_panel_width_anim`, `chat_panel_width_mode`, `chat_sheet_anim`, `chat_panel_width_snap`, `chat_info_prompt_expanded`, `chat_surfaces`, `chat_panel_tab`, `chat_panel_visible_target`, `chat_panel_open`, `chat_panel_maximized`, `chat_panel_focus`, `chat_panels_by_tab`, `chat_files`, and `panel_chat_by_tab` into a plain `SidePanelState` (or `ChatSidePanelState`) struct owned as a single field on `NativeRoot`.
2. **Sidebar-preview state** (under `crates/runner-app/src/surfaces/app_shell/`): move `sidebar_preview_open`, `sidebar_preview_peeking`, `sidebar_preview_trigger_hovered`, `sidebar_preview_dwell`, and `sidebar_preview_dwell_generation` into a plain `SidebarPreviewState` struct owned as a single field on `NativeRoot`.

Keep cross-surface coordination on `NativeRoot`.

## Read first

- `AGENTS.md` (`Test Scope` mechanical-refactor rubric, `Commit And PR Conventions`, `Crew Missions`)
- `crates/runner-app/src/surfaces/root/mod.rs` (`NativeRoot` struct) and `crates/runner-app/src/surfaces/root/init.rs`
- `crates/runner-app/src/surfaces/side_panel.rs`
- `crates/runner-app/src/surfaces/panes/header.rs` and `crates/runner-app/src/surfaces/panes/tab.rs`
- `crates/runner-app/src/surfaces/app_shell/sidebar.rs` and `crates/runner-app/src/surfaces/app_shell/actions.rs`

## Mechanical-refactor boundaries and non-goals

This mission is strictly a **mechanical refactor** under `AGENTS.md#test-scope` so no live QA is required. Every constraint below is a hard boundary:

- **Plain Rust structs only.** Do NOT introduce new GPUI `Entity` types, split GPUI views, or change `cx.notify()`, `cx.observe()`, subscriptions, `FocusHandle` creation/wiring, element render/hit-test trees, animation timing, or async/timer lifetimes. Keeping `chat_panel_focus` and `chat_files` inside a plain struct owned directly by `NativeRoot` preserves exact GPUI ownership.
- **No behavior change.** Every observable behavior, saved setting, layout calculation, and animation remains identical.
- **Preserve all test assertions.** Update field paths in existing tests to reach the new struct fields, without deleting or weakening any assertion.
- **Out of scope:** Other `NativeRoot` clusters (panes/splits, usage/accounts, chat lifecycle — tracked in #1429), `UnifiedViewer`/Changes splits (#1430), and any changes outside `crates/runner-app/`.

## Validation and handoffs

- **Coder (`@coder`):** Implement the mechanical extraction cleanly, verify with `make verify` (all workspace checks, tests, clippy, and fmt-check green), and hand off the commit/diff to `@reviewer`. Never run the desktop app.
- **Reviewer (`@reviewer`):** Review the diff in full against `AGENTS.md`'s mechanical-refactor rubric. Verify zero changes to GPUI `Entity`/notify/observe/subscription/focus/render/timer wiring, zero behavior changes, and no weakened test assertions. If anything crosses a reactivity or ownership boundary, flag it as must-fix to keep the extraction purely mechanical. Confirm no live QA is needed under `AGENTS.md#test-scope`. Never run the desktop app.
- **PR handoff:** Once review is clean, `@coder` folds this brief into the implementation commit (no standalone brief commit), pushes `refactor/1428-native-root-side-panel`, opens a PR against `main` referencing `Closes #1428` with labels `frontend-only`, `no-behavior-change`, and `risk-low` (no `needs-live-test`), drives CI green on all platforms, and has `@reviewer` add the `reviewed` label with a PR comment naming the reviewed head SHA.

## Authorization

Approved by Jason on 2026-11-24 for the `agy duo` crew (`coder` + `reviewer`, 2 slots). Authorized to commit, push, open a pull request against `main`, and drive CI green; stop at an open, reviewed, CI-green PR and do not merge.
