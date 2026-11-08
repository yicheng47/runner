# #1219 — Side panel state per tab

Jason approved the codex trio for [#1219](https://github.com/yicheng47/runner/issues/1219) on 2026-11-08. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1219-side-panel-per-tab` on `fix/1219-side-panel-per-tab`, based on `origin/main` at `f33c762873fb70d33a4824b7dff056fd4cabf0c2`.

## Problem

The side panel's state is shared across tabs. Opening Files, opening a file or ⌘-clicking a path in one tab opens the panel on Files in every other tab, widened as Files widens it. Collapsing the panel in one tab collapses it everywhere. Two pieces of state are wider than a tab:

- **Open or collapsed** is the app-wide setting `chat_panel_open` (`mission_rail_open` for missions).
- **Which surfaces are open (Info, Files) and which one shows** is one `OpenSurfaces` per window: `root.chat_surfaces` for chat and terminal tabs, and the mission workspace's `rail_surfaces` for missions.

Starting a new chat also calls `chat_surfaces.reset(...)`, which wipes the surfaces of every other tab in the window. The Files view's contents are already per subject: `FilesPanel` keeps one `FilesView` per session or mission, so file tabs need no change.

## Outcome (Jason, 2026-11-08)

Each tab keeps its own side panel state: whether its panel is open or collapsed, which surfaces it has open, and which one shows. This applies to chat tabs, terminal-only tabs and missions alike. An action in one tab affects only that tab. Panel, file-viewer and tree widths stay global settings.

## Read first

- `crates/runner-app/src/surfaces/side_panel.rs`: `OpenSurfaces`, `PanelSurfaces`, `shown`, `tabs`.
- `crates/runner-app/src/surfaces/root/mod.rs`: `chat_surfaces`, `chat_files` and `panel_chat_by_tab`, an existing per-tab map keyed by tab id.
- `crates/runner-app/src/surfaces/panes/header.rs`: `show_link_preview`, `chat_panel_layout`, `render_chat_side_panel` and its collapse handler; `crates/runner-app/src/surfaces/panes/tab.rs` for the toggle; `side_panel_open` in `panes/mod.rs`.
- `crates/runner-app/src/surfaces/start_chat/launch.rs`: the `chat_surfaces.reset` on a new chat.
- `crates/runner-app/src/surfaces/root/store.rs`: `close_off` when Settings turns a surface off.
- `crates/runner-app/src/surfaces/mission_workspace/`: `rail_surfaces`, `rail.rs` (`show_link_preview`, collapse), `view.rs` (`mission_rail_shown`). In `remote.rs`, `SavedPresentation.rail` already keeps a remote client's rail surfaces per mission: follow that precedent for local missions.

## Deliverables

1. **Per-tab surfaces.** A chat or terminal tab keeps its own `OpenSurfaces`, keyed by tab id like `panel_chat_by_tab`. A mission keeps its own, keyed by mission id. Keep one `PanelSurfaces` per window for the `+` menu and focus handles if that is simpler. Only the open/active surfaces need to be per tab. A split tab of a chat and a terminal still has one state, and still shows the last focused chat while the terminal has focus.
2. **Per-tab collapse.** Each tab and each mission stores whether its panel is open. A tab with no state of its own, such as a new tab or any tab after a restart, starts from `chat_panel_open` (`mission_rail_open` for missions). Toggling a tab's panel stores the choice for that tab and also writes the setting, so the next new tab follows the last choice. The panel's open/close animation runs only when the shown tab's state changes, not on a tab switch.
3. **Actions stay in their tab.** These open or change only the tab they act in:
   - the toggle and the collapse button;
   - `+` and the surface list;
   - ⌘-click on a path;
   - ⇧⌘O;
   - starting a new chat, which resets only the new tab's surfaces.
4. **Settings.** Turning Info or Files off in Settings closes that surface in every tab's state, as `close_off` does today. Turning **Show the side panel** off still hides every panel.
5. **Cleanup.** Drop a tab's entry when the tab closes, and a mission's entry when the mission is archived or removed, if the existing close paths make that easy. A leftover entry is harmless.
6. **Docs.** In `docs/guide/guides/chats-and-windows.md`, under the side panel and Files sections, say that each tab keeps its own panel: whether it is open, its surfaces and which one shows. Do the same for missions in `docs/guide/concepts/missions.md`. Update both zh-CN counterparts, then run `python3 script/check-guide-translations.py`.

**Non-goals:**
- No persistence of per-tab panel state across a restart: tabs start from the setting, as above.
- No change to widths.
- No change to the Files view's own state.
- No change to remote-client behaviour.

## Tests

- Replace `terminal_tab_panel_holds_files_alone_and_shares_the_chat_panel_state` in `panes/mod.rs` so that it asserts the new contract instead of the shared state.
- **Two chat tabs:**
  - Opening Files and a file in A leaves B's surfaces, active surface, width and open state unchanged.
  - Switching back to A shows Files with its file.
- **Collapse:** collapsing B leaves A open, and a new tab starts from the last choice.
- **Link:** a ⌘-click link in A opens only A's panel on Files.
- **New chat:** starting a new chat does not reset A's surfaces.
- **Missions:** two missions keep separate rail surfaces and open states.
- **Settings:** turning Files off closes it in every tab's state.
- **Gates:** run the `runner-app` tests, workspace Clippy with warnings denied, and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa)

Use the development app from this worktree. Test chats use an inexpensive model at low effort with tiny prompts, each in a fresh folder under `/tmp` with a small file to open. Missions can be no-crew test missions (`runner-dev mission start --title … --cwd /tmp/… -q`). Take the QA lock as AGENTS.md describes.

- **L1, Files stays in its tab (the reported bug):**
  - Set up two chat tabs, A and B, with B's panel on Info.
  - In A, open Files and a file.
  - Switch to B: it shows Info at the standard width.
  - Switch back to A: Files shows its file, widened.
- **L2, collapse per tab:**
  - Collapse B's panel: A's stays open, and B's stays collapsed across switches.
  - Then a new tab (⌘N) starts with the last choice.
- **L3, a link opens only its tab:**
  - In a terminal-only tab, print a path to the folder's file and ⌘-click it: that tab's panel opens on Files with the file.
  - Chat tab B is unchanged.
- **L4, a new chat keeps other tabs' surfaces:** with A on Files, start a chat in a new tab, then return to A: Files is still open and shown.
- **L5, missions:**
  - Open Files in one test mission's rail and collapse the other's.
  - Switching between them keeps each one's state.
- **L6, split tab:** in a tab that splits a chat with a terminal, focusing the terminal keeps the panel open on the chat's surfaces.

The verdict gives pass or fail per check, with screenshot evidence. The PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1–L6. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR carries `frontend-only` and `risk-medium`, and keeps `needs-live-test` until L1–L6 pass.

## Authorization

The codex trio may commit, push this branch and open one PR against `main`. It does not merge.
