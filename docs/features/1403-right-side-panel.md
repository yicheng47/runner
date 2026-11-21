# 1403 — Right Side Panel Redesign (Option O + S2 + C1)

> Tracking issue: [#1403](https://github.com/yicheng47/runner/issues/1403) (folds [#1331](https://github.com/yicheng47/runner/issues/1331) and [#1360](https://github.com/yicheng47/runner/issues/1360)).
> Design: `runner/desktop/specs/1403-right-side-panel.pen` in `yeelinks_design` (signed off 2026-11-21 at `646a913`). Boards: `t4Our` (Sign-off — Option O + S2), `i7bs0w` (Files & Changes deep-dive, C1), and reference `o0nOb` (split-screen S1 vs S2 vs S3 Info trigger comparison).

## Motivation

Runner's right side panel currently groups lightweight session metadata (**Info**) inside the same tab-and-`+`-menu container (`surfaces/side_panel.rs`) as heavy workspace tools (**Files** and **Changes**), and opens new chats onto an empty "Open a surface" panel while flipping a global `chat_panel_open` default whenever any tab collapses. Meanwhile, **Files** opens immediately into a wide two-column layout with a blank viewer area before any file is chosen, and **Info** repeats `cmd`, `cwd`, and `branch` in worktrees.

This redesign separates lightweight session/mission **Info** from the workspace-tools right panel and adopts an Orca-style top icon activity bar with compact 300 px project-tree and changes-list defaults that widen progressively only when a file or diff is opened.

## Design Contract

### 1. Orca-style Top Activity Bar (Right Panel Header)

- Replace the surface tab strip, `+` surface menu, and empty "Open a surface" view with a fixed 40 px **top icon activity bar** on the left of the panel header, plus `Maximize` (`⤢`, shown only when widened on a file/diff or already maximized) and `Collapse` (`panel-right-close`) on the right.
- Mode icons by surface (subject to Settings toggles; `Sessions` is always enabled on missions):
  - **Chat & Terminal:** `Files` (`files.svg` / folder icon), `Changes` (`git-branch` / `git-compare` with a subtle amber dirty dot when changes exist). Future tools such as an embedded browser (#1260) append cleanly to this strip without adding a `+` menu.
  - **Mission:** `Sessions` (`users.svg`, first and default), `Files`, `Changes`.
- The active icon uses a subtle raised tile (`theme::raised()`) and a 2 px bottom accent indicator (`theme::accent()`). Clicking an icon switches modes in one click; each mode preserves its own state while the panel stays open.
- If a user disables every workspace tool in `Settings → Side panel` on a chat/terminal tab (or turns off `Show side panel`), the right panel and its header open toggle are hidden cleanly; a mission always keeps `Sessions` while `Show side panel` is on.

### 2. Default Open State & Progressive Widening (C1)

- **Default mode & width:** On wide windows, new chat and terminal tabs start **open on `Files` at 300 px** showing the **Project Tree only** (no blank file-viewer column beside it). Missions start **open on `Sessions` at 300 px** (retaining the existing rail width clamping/resize).
- **Files default (300 px Project Tree):** Root picker (`runner ▾`) + tree actions (`collapse all`, `refresh`, `⋯`), segmented `All files | Changed (N)` filter toggle, inline `Find files` (`⇧⌘O`) trigger/filter, and the project tree. Clicking a file in the tree (or `⌘`-clicking a terminal file link / picking in `⇧⌘O`) widens the panel smoothly to **~660–700 px** (file preview + open file tabs + `Diff` / `Rendered|Source` / `Open in` / `Tree` toggle + `⌘F` find bar beside the docked project tree). Closing the last file tab shrinks smoothly back to the **300 px Project Tree**.
- **Changes progressive widening (C1):** Clicking `Changes` in the activity bar keeps the panel compact at **300 px**, showing the full `#1345` Changes list rail (Checkout/branch picker, `Base` picker, Commits scope `All changes` / `Uncommitted changes` / recent commits, summary + Flat/Tree toggle, and changed-files list with `M`/`A`/`D` and `✓` viewed markers). Clicking a changed file widens the panel to **~740 px** (Diff viewer with `Unified | Split`, `✓ Viewed`, `Open in`, and `×` close-diff button beside the docked Changes rail). Clicking `×` on the diff header closes the diff and shrinks back to the **300 px Changes list**. Picking `Split` when narrow continues to maximize across the workspace as in `#1141`.
- **Edge states at 300 px:** Terminal tabs without a folder show the compact "Choose a folder to show" picker inside the 300 px Files rail; remote chats/terminals/missions show the clean 300 px unavailable notice in Files/Changes while keeping Sessions (on remote missions) and S2 Info working.

### 3. S2 Title `▾` Info Popover (`⌘I`) & De-duplicated Info Rows (#1360)

- Remove `Info` from the right side panel completely. Do **not** add an `(i)` icon button to chat or mission headers.
- Integrate the compact **Info card** at the top of each chat pane's existing **Title `▾` popover** (`pane_title_menu`) above a divider and the existing pane actions (`Rename chat`, `Pin tab`, `Archive chat`, etc.). Every chat pane (single-pane, split-pane, archived, and remote) owns its own Title `▾` Info popover directly in the window popup layer (`popup_layer`), flipping above if anchored near the bottom of a split so it never clips. Plain shell/terminal panes keep their existing lightweight title menu without an agent Info card.
- Apply the same **S2 Title `▾` popover** pattern to **Mission headers**: clicking the mission title trigger opens the compact **Mission Info card** (`Goal`, `worktree` / `cwd`, `permissions`, `started`, `mission_id` with copy) above a divider and the existing mission actions (`Rename`, `Stop all`, `Archive`).
- **Keyboard shortcut (`⌘I` on macOS, `Ctrl+Shift+I` on Windows/Linux):** Add a configurable keymap action (`toggle-session-info`, default `cmd-i` / `ctrl-shift-i`, context `!TextInput && !Settings`) that opens/toggles the focused chat pane's Title `▾` Info popover (or the Mission Title `▾` popover on a mission workspace). `Esc` or outside click dismisses it and restores focus to the terminal/workspace.
- **De-duplicated Info rows (#1360):**
  - When the session or mission folder is inside a linked worktree, replace redundant `cwd` and `branch` rows with a single **`worktree`** row showing the branch name, a copy button (copies full worktree path; tooltip shows full path), and the `from <base>` worktree marker below it.
  - Show **`cwd`** only when outside a linked worktree (main checkout or non-git folder) OR when the session's folder is a subdirectory below the worktree root (shown relative to the worktree root).
  - Show **`cmd`** only when the session's executable differs from the runtime's default command (e.g., a custom wrapper/binary configured on the role); hide default `claude`/`codex`/`pi`/`agy`/`opencode` commands.
  - Keep `Role` / `Open role ↗` (for role chats), `Runtime · model · effort` (and `Account` when multiple Claude accounts exist), `session_key` with copy, and a collapsed-by-default **`▸ System prompt`** toggle (with line/size meta) that expands in-place into a scrollable Markdown container (`max-height ~180 px`) without pushing the actions below off-screen. Remote chats show the metadata currently carried by the host's remote overview (`runtime`, host `worktree`/`from <base>` while connected, and `cwd` outside a worktree or as a relative subfolder); host-only fields not carried by the remote overview (`model`, `effort`, `session_key`, `system_prompt`) remain out of scope under this feature's `frontend-only` contract.

### 4. Per-Tab State, Split-Screen, and Narrow-Window Overlay Sheet (#1331)

- **Per-tab open/mode independence:** Opening or collapsing a tab's right panel updates only that tab (or mission) — it no longer overwrites a global `chat_panel_open` / `mission_rail_open` default for future tabs. Each tab remembers its own open/closed intent, active activity-bar mode, and open files/diff while in memory.
- **Narrow windows & split auto-sheet:**
  - Single-pane minimum main width remains `MAIN_MIN_WIDTH` (`420 px`). Split-pane tabs enforce a per-column minimum of `340 px` (`split_columns * 340 px`, where `split_columns` is the maximum horizontal column count in the active pane tree).
  - On a **narrow window** where `available_width - 300 < min_main_width`, new tabs start with the panel **collapsed** so the terminal keeps full width; toggling the panel open (or invoking `⇧⌘O` / `⌘`-click file link) slides the panel over the right edge of the workspace as an **elevated overlay sheet** (subtle backdrop scrim + drop shadow, `Esc` or click-outside scrim or collapse button closes it, zero terminal `SIGWINCH`).
  - When a tab is docked at **300 px** (for example, a 2-column split on a 1440 px screen) and clicking a file or changed file would widen the panel such that `available_width - widened_width < min_main_width`, the widened viewer automatically presents as an **elevated overlay sheet** over the right edge of the split panes instead of crushing the split terminals; closing the last file/diff snaps smoothly back to the docked 300 px rail.
  - Widening the window back above the threshold restores docked presentation seamlessly without losing open tabs or state.
- **Split-screen "Following" cue:** The tab's right panel continues to follow the focused chat pane (or last focused chat pane when a shell pane in the same split has focus). When a split tab contains panes in different folders/worktrees, show a quiet `Following <pane label> · <branch/folder>` badge under the Project Tree / Changes root header so focus-driven root switches are unmistakable.

### 5. Settings → Side Panel

- Update `Settings → Side panel → Panel`:
  - Keep `Show side panel` master switch (updated description noting Project Tree default on wide windows, auto-sheet on narrow windows, and per-tab state).
  - Remove the `Info` surface toggle (and drop `info_view` from visible settings; tolerate legacy `info_view` in saved settings JSON via `#[serde(default)]` / serde ignore so upgrades never fail).
  - Keep `Files` and `Changes` toggles without the old single-letter `+`-menu keycaps.
  - Add a quiet footer note explaining that Session and Mission Info live in each pane's `Title ▾` menu (`⌘I` / `Ctrl+Shift+I`).
- Update EN and zh-CN i18n catalogs and Settings search terms accordingly.

## Implementation Phases

1. **Panel state & activity bar (`surfaces/side_panel.rs`, `app_settings.rs`):** Replace `OpenSurfaces` / `+` menu / empty list with `PanelMode` (`Sessions`, `Files`, `Changes`), Orca-style 40 px activity bar renderer, per-tab open state (decoupled from mutating global open flags), and docked-vs-sheet geometry helper (`MAIN_MIN_WIDTH = 420`, split column min `340`).
2. **Files & Changes progressive widening (`surfaces/files/`, `surfaces/changes/`):** Keep Files at 300 px tree-only until a file opens (adding `All files | Changed (N)` segmented toggle and inline `Find files` trigger in the 300 px tree header; shrinking back to 300 px when the last tab closes). Keep Changes at 300 px list-only (C1) until a changed file is clicked (adding `×` close-diff button in diff header to shrink back to 300 px). Add the split-pane `Following <pane>` indicator when panes differ.
3. **S2 Title `▾` Info popover & `#1360` de-duplication (`surfaces/panes/`, `surfaces/mission_workspace/`, `keymap.rs`):** Embed the compact `#1360` Info card (with collapsible inline scrollable System prompt) at the top of chat `pane_title_menu` and mission header title menu, wire `toggle-session-info` (`⌘I` / `Ctrl+Shift+I`), and preserve full behavior on local, archived, split, and remote workspaces.
4. **Overlay sheet presentation & Settings/Docs (`surfaces/root/`, `surfaces/settings/page/side_panel.rs`, `docs/guide/`):** Render elevated overlay sheet with scrim when narrow or split-widening exceeds width budget; simplify `Settings → Side panel`; update EN + zh-CN guide pages (`chats-and-windows.md`, `missions.md`, `sessions.md`, `keyboard-shortcuts.md`).

## Verification

### Automated Tests

- Unit and GPUI app tests covering:
  - Activity bar mode switching (`Files`, `Changes`, `Sessions` on missions) and Settings toggle visibility (including all tools off and legacy settings deserialization).
  - Default 300 px tree-only Files state → widen on file open → shrink back to 300 px on closing last file; same C1 progression for Changes (`list → diff → close diff → 300 px list`).
  - Per-tab open/closed isolation (collapsing tab A does not collapse new tab B) and narrow-window / split-widening docked-vs-sheet layout math (`420 px` single / `340 px` per split column).
  - S2 Title `▾` Info popover on role chats (collapsed & expanded System prompt), runtime chats, remote chats, split panes (independent per-pane metadata), and missions, plus `#1360` worktree/cwd/default-cmd de-duplication and `⌘I` shortcut dispatch.
- Any new daemon-backed or async app tests must follow `AGENTS.md`'s no-race rules (`h.wait(...)`, no fixed sleeps) and pass 20 consecutive runs via `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`.

### Live Checks (macOS development app, `@qa` with `script/qa-lock`)

1. **Chat default & Files progressive widening (`WS-FILES-01` / feature):** Open a new chat in a wide window → right panel opens on 300 px Project Tree (no "Open a surface", no blank file viewer), activity bar shows Files active + Changes dirty dot. Use `All files | Changed` and `Find files (⇧⌘O)`, click a file → panel widens to ~660–700 px with tabs + Diff + Open in + Tree, test `⌘F` and Markdown `Rendered | Source`, close last tab → shrinks back to 300 px tree.
2. **Changes C1 progressive widening (`WS-CHANGES-01`, `WS-CHANGES-09`):** Click Changes icon → stays 300 px with Checkout/Base pickers, Commits scope, and changed files list. Click a changed file → widens to Unified diff with `Viewed ✓` and `×`; click `Split` → maximizes cleanly and `Restore` returns; click `×` on diff header → shrinks back to 300 px Changes list.
3. **S2 Title `▾` Info popover & `#1360` de-duplication (single + split + remote):** In a worktree role chat, verify no `(i)` button in header; click `Title ▾` and press `⌘I` → compact Info card shows `worktree` (without duplicate `cwd`/`branch` or default `cmd`), `session_key` copy, expandable scrollable System prompt, and `Rename / Pin / Archive` below. In a 2-pane split with different worktrees, verify each pane's `Title ▾` opens its own Info, bottom/narrow pane popovers stay unclipped, and the docked tree shows `Following <pane>`. Verify a remote chat on the same-Mac two-host relay fixture opens S2 Info cleanly and shows the compact remote unavailable notice in Files/Changes.
4. **Per-tab isolation & narrow/split overlay sheet (#1331):** Collapse the panel in Tab A, open Tab B → Tab B still opens normally. Resize window narrow (`main < 420 px`) → panel collapses/opens as an elevated overlay sheet with scrim without crushing the PTY; widen a file inside a 2-column split → auto-sheets and snaps back to 300 px tree when closed.
5. **Mission workspace & Light Mode (`SET-PANEL-01`, `SET-THEME-01`):** Open a mission → activity bar opens on `Sessions` (300 px) with `Files` and `Changes` 1 click away; clicking Mission `Title ▾` shows Mission Info + actions. Verify `Settings → Side panel` drops `Info` and toggles `Files`/`Changes` cleanly, and verify Dark + Runner Light visual contrast.
