# Brief: #1421 — Unified Files & Changes Viewer + Stable Per-Tab Panel Width (Option A)

## Starting point

- **Issue:** [#1421](https://github.com/yicheng47/runner/issues/1421).
- **Branch / worktree:** `feat/1421-unified-files-changes` at `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1421-unified-files-changes`.
- **Base commit:** `70e3480194126408995438646764fdd1fb80d7bd` (`origin/main`).
- **Parallel isolation:** Sibling worktrees exist at `.worktrees/fix-1395-remote-project-drag` and `.worktrees/test-flaky-process-probe-tests`. Stay strictly inside this worktree, never assume unmerged peer branches landed, and never share `CARGO_TARGET_DIR` (if `coder` builds while `qa` holds a candidate build, use `CARGO_TARGET_DIR=target/coder` inside this worktree).

## Problem and outcome

Post-#1403 (`70e34801`), switching between `Files`, `Changes`, and `Sessions` in the right side panel jumps the panel width (`300 / 680 / 740 px`) and resizes the terminal PTY because width is derived from the active activity-bar mode (`file_viewer_width` `440` vs. `changes_viewer_width` `500` vs. `Sessions` `300`) and `FilesView` and `ChangesView` each host separate, decoupled viewers.

Implement the signed-off **Option A — Shared Docked Viewer + Fixed-Width Right Rail**:
- **Authoritative spec:** `docs/features/1421-unified-files-changes.md` on this branch (wins on every behavioral detail).
- **Signed-off design:** `~/repos/yeelinks/yeelinks_design/runner/desktop/specs/1421-unified-files-changes.pen` (commit `01fb5c7`): boards `uvxUZ` (Option A section `hHBbi` + Mission proof `C2`) and `C8P7J` (1:1 unified viewer Diff ↔ Code deep-dive, Runner Light `TVJPT`, and width contract `O2iqVZ`).

## Read first

1. `docs/features/1421-unified-files-changes.md` (full contract, edge cases, and live checks).
2. `crates/runner-app/src/surfaces/side_panel.rs` (`compute_panel_layout`, `activity_bar`, `PanelLayout`, `Widened`, `SavedPanel`).
3. `crates/runner-app/src/surfaces/files/mod.rs` (`FilesPanel::panel_layout`, `files_widening`, `changes_widening`, `DirtyTracker`, `KEPT_VIEWS` saved state).
4. `crates/runner-app/src/surfaces/files/panel_view.rs`, `crates/runner-app/src/surfaces/files/open_file.rs`, and `crates/runner-app/src/surfaces/files/open_file/render.rs` (tab strip, `CodeView`, `MarkdownView`, inline diff toggle, find bar, `Open in`).
5. `crates/runner-app/src/surfaces/changes/mod.rs` and `crates/runner-app/src/surfaces/changes/render.rs` (Checkout/Base/Commits/Changed-files rail, `UnifiedDiffView`, `✓ Viewed`, keyboard navigation).
6. `crates/runner-app/src/surfaces/panes/header.rs`, `crates/runner-app/src/surfaces/panes/tab.rs`, `crates/runner-app/src/surfaces/mission_workspace/rail.rs`, `crates/runner-app/src/surfaces/mission_workspace/view.rs`, and `crates/runner-app/src/app_settings.rs`.

## Deliverables and non-goals

1. **Aligned 40 px header row & decoupled right rail:** When a document tab is open, render `[Unified Viewer (flex_1, $bg)]` left and `[Fixed Right Rail (file_tree_width, default 240 px, $panel)]` right under one aligned `40 px` top row — viewer tab strip + `Code | Diff` + `Pin` + rail toggle + `Maximize` above the viewer, and Orca icon activity bar (`Files`, `Changes` with dirty dot; `Sessions` first on missions; `Collapse`) strictly above the right rail. Keep `Collapse` + `Show rail` reachable when the rail is hidden. No dead Browser placeholder icon.
2. **Per-tab fixed panel width (0 px jump on rail switch):** Derive widened state from the subject's unified viewer open state, never from active `PanelViewId`. Switching `Files ↔ Changes ↔ Sessions` swaps only the right rail (`0 px` panel width change, zero PTY `SIGWINCH`, open viewer tabs preserved). Merge `file_viewer_width` and `changes_viewer_width` into one unified `file_viewer_width` preference (default `460 px`, tolerant of legacy `changesViewerWidth` in JSON). Preserve `#1420` default-collapsed chat/terminal state, per-tab isolation, and `#1403` narrow/split auto-sheet protection (`420 px` / `340 px`).
3. **Unified File + Diff viewer:** Both `Files` and `Changes` feed one shared per-subject viewer:
   - `Files` (tree, `⇧⌘O`, Markdown link, `⌘`-click terminal link) opens pinned tabs; `Changes` single-click / `↑↓` / `j`/`k` swaps one italic preview tab (focusing an existing pinned tab if already open; promoted to pinned via `Pin`, double-click, `Diff → Code`, or opening from `Files`).
   - In-place `Code | Diff` toggle on changed files (preserving line/scroll context where available; disabled cleanly on deleted files), shared `Unified | Split` preference + auto-maximize below `360 px`/side, live `✓ Viewed` synced with `Changes` rail, `Open in`, `⌘F`, and Markdown `Rendered | Source`.
   - Keep `Files` and `Changes` live watch refresh and `DirtyTracker` clean without duplicate watchers or leaks when switching to `Sessions`, collapsing, or switching tabs.
4. **i18n and User Guides:** Update `crates/runner-app/i18n/{en,zh-hans}.toml` and both EN + zh-CN guides (`docs/guide/guides/chats-and-windows.md`, `docs/guide/zh-CN/guides/chats-and-windows.md`, `docs/guide/concepts/missions.md`, `docs/guide/zh-CN/concepts/missions.md`).

**Non-goals:** Strictly `frontend-only` (`crates/runner-app/` only — zero changes to `runner-core`, `runner-daemon`, `runner-cli`, protocol, or SQLite). Do not implement Option B (main-area tabs), Option C (merging `Files`/`Changes` icons), embedded browser (#1260), or inline diff comments (#1293).

## Validation and handoffs

- **Automated (`@coder` before review):** Add/update deterministic unit and GPUI app tests covering per-tab width stability across `Files ↔ Changes ↔ Sessions`, single unified viewer width resizing & legacy JSON compatibility, italic preview tab vs. pinned tab promotion, in-place `Code | Diff` flipping + `✓ Viewed` sync + deleted-file guard, narrow/split auto-sheet geometry, and watch/dirty-tracker lifecycle. Loop new/modified async app tests 20x via `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'` and report the count. Run `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `python3 script/check-guide-translations.py`, and `git diff --check`.
- **Review (`@reviewer`):** Reviews frozen commits against `docs/features/1421-unified-files-changes.md` and `1421-unified-files-changes.pen` (`uvxUZ`, `C8P7J`), checks test determinism (no races/fixed sleeps), confirms the 5 live QA checks, and applies `reviewed` on the final PR head.
- **Live QA (`@qa` after clean review, macOS dev app):** Acquire `script/qa-lock acquire --timeout 540` (preserve any pre-existing user dev sessions), run all 5 live checks in `docs/features/1421-unified-files-changes.md#live-checks-macos-development-app-qa-with-scriptqa-lock`, record evidence in `docs/tests/runs/1421-unified-files-changes-mac.md` and the PR description, and release `script/qa-lock`. Keep `needs-live-test` if any required check fails/blocks or for untested Windows/Linux native checks per `AGENTS.md`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). Authorized to commit, fold this brief and `docs/features/1421-unified-files-changes.md` into clean implementation commit(s) before final push, push `feat/1421-unified-files-changes`, open a PR against `main` (`Fixes #1421`, `frontend-only`, accurate `risk-*`), and drive CI green. Stop at the open, reviewed, QA-verified, CI-green PR; do not merge.
