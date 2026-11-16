# Mission Brief: Keep right side panel width when closing or switching tabs (#1344)

**Tracking issue:** [#1344](https://github.com/yicheng47/runner/issues/1344)
**Branch:** `feat/1344-side-panel-width` | **Base:** `origin/main` (`69ba26d0`)
**Worktree:** `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1344-side-panel-width`
**Crew:** `carbon trio` (`coder`, `reviewer`, `qa`)

**Parallel isolation:** No other active worktrees are running right now. Stay strictly inside `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1344-side-panel-width`, never edit the root checkout `/Users/jason/repos/yicheng47/runner`, and take `script/qa-lock acquire --timeout 540` before launching the development app during live QA.

## Authoritative Spec

- **Feature spec:** `docs/features/1344-side-panel-width.md` (indexed in `docs/features/README.md`).
- **Commit hygiene:** Keep `docs/features/1344-side-panel-width.md`, `docs/features/README.md`, and `docs/impls/briefs/1344-side-panel-width.md` folded cleanly into the implementation commit before final push so no standalone brief commit remains in PR history.

## Read First

1. `docs/features/1344-side-panel-width.md`
2. `crates/runner-app/src/surfaces/files/mod.rs` (`FilesSurface`, `FilesPanel::widening`, `layout_with_maximize`, `changes_layout_with_maximize`) and `crates/runner-app/src/surfaces/files/panel_view.rs` (`FilesView::wide`, `FilesView::widening`).
3. `crates/runner-app/src/surfaces/panes/header.rs` (`chat_panel_layout`, `sync_chat_panel`) and `crates/runner-app/src/surfaces/panes/tab.rs` (`ChatPanelResizeDrag`).
4. `crates/runner-app/src/surfaces/mission_workspace/rail.rs` (`rail_layout`) and `crates/runner-app/src/surfaces/mission_workspace/view.rs` (`MissionRailResizeDrag`).
5. `crates/runner-app/src/surfaces/side_panel.rs` (`Widened`, `widened`, `OpenSurfaces`, `SavedPanel`, `PanelSurfaces`).
6. `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md` (line ~39 on `Files` widening and closing the `Files` tab).

## Deliverables & Non-Goals

1. **Retain widened side-panel width on tab close and tab switch (`runner-app` only):**
   - A brand-new Chat/Terminal tab's side panel or a freshly entered/reset Mission rail (`OpenSurfaces::default()` / `PanelSurfaces::reset(Default::default())`) starts at its `standard` width (`chat_panel_width` / `mission_rail_width`) before `Files` or `Changes` has widened it.
   - Once `Files` or `Changes` widens that panel, retain its widened state (`Some(Widened)`) for that tab's panel (including across workspace tab switches via `SavedPanel`) or that mission's rail across:
     - Switching between side-panel surface tabs (`Sessions`, `Info`, `Files`, `Changes`), even when `Files` has no open file tabs or when switching away from `Changes`.
     - Closing any side-panel surface tab (`Files`, `Changes`, `Info`), including closing the last surface tab down to the empty `Open a surface` list (`shown.is_none()`).
     - Closing the last open file tab inside `Files` (whether the tree toggle is on or off).
   - Keep `MAIN_MIN_WIDTH = 420.` protection for the chat/mission area when the window narrows, keep `#1141 m3` side-panel maximize intact, and keep outer-divider drag (`ChatPanelResizeDrag` / `MissionRailResizeDrag`) sizing smoothly via `widened` while in the retained-wide state so dragging on `Info`, `Sessions`, or `Open a surface` never snaps down to `CHAT_PANEL_MAX` / `MISSION_RAIL_MAX` (`480 px`).
2. **Automated tests & bilingual guide docs:**
   - Update existing unit/visual tests in `crates/runner-app/src/surfaces/{files/tests.rs,panes/tests.rs,mission_workspace/tests.rs}` that asserted closing `Files` or switching away without an open file tab shrank the panel back to `288.` / `320.`.
   - Add regression tests covering Chat and Mission panel width retention across surface tab switches, surface tab closes (`Files`, `Changes`, `Info`, and down to `Open a surface`), last file-tab close, and outer-divider drag on `Info` / `Sessions` after widening.
   - Update `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md` and verify `python3 script/check-guide-translations.py`.
3. **Explicit non-goals:**
   - No daemon, CLI, protocol, or schema changes (`frontend-only`).
   - Do not change default `chat_panel_width`, `mission_rail_width`, `file_viewer_width`, or `file_tree_width` constants in `app_settings.rs`.

## Role Boundaries, QA & PR Gate

- **`coder`:** Implement, run `cargo test -p runner-app`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `python3 script/check-guide-translations.py`, commit cleanly, and hand off to `reviewer`. Do not run the live app.
- **`reviewer`:** Review the diff against `origin/main`, verify the spec and tests, and confirm the live checks for `qa`.
- **`qa`:** Acquire `script/qa-lock acquire --timeout 540`, run the selected macOS live checks in the development app (`com.wycstudios.runner-dev`, preserving any pre-existing user sessions and archiving only QA-created sessions/missions), record the run in `docs/tests/runs/1344-side-panel-width-mac.md`, and release `script/qa-lock release` after cleanup.
- **PR & Labels:** Once `reviewer` and `qa` pass (`NO REMAINING MUST-FIX ISSUES` / `NO REMAINING MUST-FIX QA ISSUES`), `coder` folds the commits cleanly (e.g. implementation+spec+guide commit + QA record commit, with no standalone brief commit), pushes `feat/1344-side-panel-width`, opens a **non-draft** PR closing `#1344` with labels `frontend-only`, `risk-low` (or `risk-medium` if appropriate), `needs-live-test` (since Windows/Linux live checks remain unrun), and `reviewed` (applied by `reviewer` on the final head SHA), and waits for all CI checks to pass `SUCCESS`. **Do not merge the PR**; report ready once CI is green.
