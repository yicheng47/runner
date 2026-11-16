# 1344 — Keep right side panel width when closing or switching tabs

Tracking issue: [#1344](https://github.com/yicheng47/runner/issues/1344)

## Motivation

Opening **Files** or **Changes** widens the right side panel (`file_viewer_width + file_tree_width`, default `440 + 240 = 680 px`, clamped so the chat or mission keeps at least `MAIN_MIN_WIDTH = 420 px`) from its narrow standard width (`chat_panel_width = 320 px` or `mission_rail_width = 288 px`). Today, the panel only stays wide while **Changes** is the shown tab, while **Files** is the shown tab, or while **Files** remains an open tab with at least one open file tab. Closing the **Files** or **Changes** surface tab, switching from **Changes** (or **Files** with no open file tab) to **Info** or **Sessions**, closing all surface tabs to the empty **Open a surface** list, or closing the last open file tab in **Files** when the tree is hidden shrinks the side panel back down to `standard`. That sudden width drop shifts the panel and the chat or mission workspace beside it and feels jumpy.

## Scope

- Once a Chat/Terminal tab's side panel or a Mission's side panel (rail) has widened for **Files** or **Changes**, keep that widened panel width across:
  - Switching between side-panel surface tabs (**Sessions**, **Info**, **Files**, **Changes**), even when **Files** has no open file tabs or when switching away from **Changes**.
  - Closing any side-panel surface tab (**Files**, **Changes**, **Info**), including closing the last surface tab down to the empty **Open a surface** list.
  - Closing the last open file tab inside **Files** (whether the file tree toggle is on or off).
- Preserve each Chat/Terminal tab's retained widened state across workspace tab switches (`SavedPanel` in `NativeRoot`). A brand-new tab or a mission whose surface state is freshly initialized (`OpenSurfaces::default()` / `PanelSurfaces::reset(Default::default())`) starts at `standard` (`chat_panel_width` / `mission_rail_width`) until **Files** or **Changes** widens it.
- Keep `MAIN_MIN_WIDTH = 420 px` protection for the chat or mission area on window narrowing, keep side-panel maximize (`#1141 m3`) intact, and keep manual outer-divider dragging (`ChatPanelResizeDrag` / `MissionRailResizeDrag`) smooth from the current widened width on any surface tab (**Info**, **Sessions**, **Files**, **Changes**, or the empty list) without snapping down to `CHAT_PANEL_MAX` / `MISSION_RAIL_MAX` (`480 px`).
- Update `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md` where they state that closing the **Files** tab returns the panel to its usual width or that only an open file keeps the panel wide on other surfaces.

## Implementation Phases

1. **Panel layout & retained widened state (`crates/runner-app/src/surfaces/{side_panel.rs,files/mod.rs,files/panel_view.rs,panes/header.rs,mission_workspace/rail.rs}`):**
   - Track whether a panel has widened (and its last effective tree-column contribution when **Files** or **Changes** is not currently supplying it) on the panel's surface state (`PanelSurfaces` / `SavedPanel`) or `FilesPanel` subject state, and keep `FilesView::widening()` active once a root has loaded even if `tree_hidden` is toggled with zero open file tabs.
   - Return `Some(Widened)` from `chat_panel_layout` and `rail_layout` whenever the panel is currently widened or has retained its widened state, so both the panel width and outer-divider drag (`ChatPanelResizeDrag` / `MissionRailResizeDrag`) stay consistent across **Info**, **Sessions**, **Files**, **Changes**, and the empty **Open a surface** state.
2. **Unit & visual tests (`crates/runner-app/src/surfaces/{files/tests.rs,panes/tests.rs,mission_workspace/tests.rs}`):**
   - Update existing assertions that expected closing **Files** or switching away without an open file tab to shrink the panel back to `288 px` / `320 px`.
   - Add regression coverage verifying that:
     - Opening **Files** (with or without an open file tab) or **Changes** widens the panel, and switching to **Info** / **Sessions**, closing **Files** / **Changes** / **Info**, closing the last open file tab, or closing all tabs to **Open a surface** retains the widened width.
     - Dragging the outer panel divider while on **Info** / **Sessions** after widening resizes smoothly without snapping to `480 px`.
3. **Guide documentation (`docs/guide/{,zh-CN/}guides/chats-and-windows.md`):**
   - Update the English and Simplified Chinese guide prose describing side-panel width retention across surface switches and tab closes, and verify with `python3 script/check-guide-translations.py`.

## Verification

- Automated tests:
  - `cargo test -p runner-app`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `python3 script/check-guide-translations.py`
- Required live checks (`docs/tests/runs/1344-side-panel-width-mac.md`):
  1. **Chat side panel width retention across tab switch & close:** In a chat tab, open **Files** (or **Changes**) so the side panel widens, switch to **Info**, close **Files** / **Changes**, close the last open file tab inside **Files**, and close **Info** down to **Open a surface** — verify the side panel stays at its widened width without jumping back to `320 px`, and dragging the outer divider on **Info** resizes smoothly.
  2. **Mission rail width retention across tab switch & close:** In a mission workspace, open **Changes** or **Files** so the rail widens (placing two **Sessions** cards per row when `>= 560 px`), switch to **Sessions** and **Info**, and close **Changes** / **Files** — verify the rail keeps its widened width and two-up card layout without shrinking back to `288 px`, while narrowing the window still preserves `MAIN_MIN_WIDTH = 420 px` for the mission area.
