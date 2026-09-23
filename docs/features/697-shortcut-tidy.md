# 697 — Keyboard shortcut tidy

> Tracking issue: [#697](https://github.com/yicheng47/runner/issues/697)
> Priority: P2, milestone 0.11. Platforms: macOS and Windows.
> Design: `design/specs/697-shortcut-tidy.pen`, frame `ntV78` (Settings — Keyboard shortcuts).
> Decisions, 2026-09-22:
> - The nine tab rows become one fixed row, "Go to tab 1–9 · ⌘1–⌘9" (Ctrl+1–Ctrl+9 on Windows), like Copy and Close pane: not rebindable.
> - Copy leaves the page for the hidden reserved list, next to Paste.
> - Stop moves from ⌘. to ⇧⌘X, and Resume gets its own key, ⇧⌘R (Ctrl+Shift+X and Ctrl+Shift+R on Windows). No single key toggles between stop and resume.
>
> Decisions, 2026-09-23 (after the first smoke test):
> - No separate "Fixed" card: the fixed rows stay in the one list, in registry order, without an Edit control.
> - A fixed shortcut can be turned off with its delete icon and restored, but never given another key. A turned-off fixed key stays reserved.

## Motivation

Settings → Keyboard shortcuts is harder to scan than it needs to be, and one of the most common session actions has no key.

- **Nine tab rows**: `select-tab-1` to `select-tab-9` ("Go to tab 1" … "Go to tab 9", ⌘1–⌘9, Ctrl on Windows) render as nine near-identical rows. They push the useful entries down the page.
- **Stop works only in chats**: "Stop focused session" (`stop-session`, ⌘.) exists, but `stop_focused_session` (`surfaces/chat.rs:554`) returns unless the route is the chat surface, so it does nothing on the mission view.
- **No resume key**: a stopped session can only be resumed with the mouse.

## Scope

### 1. One row for switching tabs by number

The Keyboard shortcuts pane (`render_shortcuts_settings`, `surfaces/settings_page.rs:1463`) renders the nine `select-tab-N` entries as one row in their place:

- **Row**: titled "Go to tab 1–9", described as "Open a visible sidebar tab or mission by its position."
- **Chip**: "⌘1–⌘9" (Ctrl+1–Ctrl+9 on Windows).
- **Controls**: those of every fixed row (section 2): delete, and restore while turned off.
- **Place**: where the nine rows were, after Reset zoom.

**Data**: the nine entries keep their ids and become `fixed: true`, and the key-binding build (`install_bindings`) keeps installing nine bindings.

**Custom keys are lost**: `effective_binding` returns the default for fixed entries, so an override of a `select-tab-N` saved before this change stops applying. This is a deliberate choice, made 2026-09-22 against the issue's "must keep working" line. The override stays in the settings file, ignored, and nothing migrates it. The release notes say it. A digit unbound before this change is ignored the same way: the nine keys are turned off and on together, and the row is off only when all nine are.

**Conflicts**: other entries can no longer take ⌘1–⌘9, because `find_conflict` already counts fixed defaults.

**Search**: the row matches its title, its description, its chip, and every digit's old title and binding, so "tab 3", "⌘3" and "go to tab" all find it.

**Where the grouping lives**: a small `keymap` helper names the group, and the settings pane skips the nine entries and renders the one row. It is not a new keymap entry.

### 2. Fixed shortcuts can be turned off, not changed

The pane renders one card, every row in registry order. The fixed rows are New window (⇧⌘N, first), Go to tab 1–9 (⌘1–⌘9, after Reset zoom) and Close pane (⌘W, after Split pane down). On Windows the chips read Ctrl in place of ⌘, as everywhere on the page.

- **Controls**: a fixed row has no Edit control and its chip does not start recording. It keeps the delete icon, and shows restore while it is turned off, in the same places as on other rows.
- **Turning off**: delete saves the fixed entry as unassigned (all nine entries for the tab row), and its chip reads "Unassigned". `effective_binding` honours that, while still ignoring a custom key saved for a fixed entry.
- **Still reserved**: a turned-off fixed key is not freed for another shortcut: `find_conflict` counts a fixed entry by its default, whatever its override.
- **Key bindings**: `install_bindings` installs ⌘W and ⇧⌘N only while Close pane and New window are on. Turning Close pane off drops ⌘W for both halves of its action, closing a split pane and closing the window.
- **Menu bar**: the macOS menu reads its shortcuts from the key bindings when it is set, and a stale one still fires, so the menus are rebuilt whenever the bindings are. File → New Window and Window → Close Window stay, without their key once it is off.
- **Reset all to defaults** counts a turned-off fixed row as a change, and clearing restores it.

**Copy leaves the page.** The `copy` entry (⌘C, "Copy the current terminal selection.") is the system shortcut, which Runner wires so that a terminal, feed or field selection copies. It is the same kind of shortcut as Paste (⌘V), which is already a hidden reserved entry. Move `copy` from `entries()` to `reserved_entries()`:

- The unconditional Copy bindings at the top of `install_bindings` do not change.
- `find_conflict` still refuses ⌘C for any other entry, because it checks reserved entries.

**Search** filters the one card, and the existing "No shortcuts match" message shows when no row matches.

### 3. Stop and resume the focused session from the keyboard

**Keys**: `stop-session` changes its default from ⌘. to ⇧⌘X, and a new entry `resume-session` defaults to ⇧⌘R. On Windows `windows_default` turns them into Ctrl+Shift+X and Ctrl+Shift+R.

- **Why this family**: macOS terminals never receive ⌘ combinations, and Windows terminal emulators keep Ctrl+Shift for app shortcuts, so neither key is taken from a program running in a pane.
- **R for resume** follows Xcode (⌘R run), JetBrains (⌘R rerun) and Zed (⇧⌘R restart kernel).
- **X for stop** follows tmux's kill-pane (prefix + x).
- **Rejected**: ⌘R, because it becomes Ctrl+R on Windows, which is reverse history search in bash, zsh and PowerShell. F5 / ⇧F5 (VS Code, Zed's debugger), because full-screen programs in a pane use F-keys. Ctrl+Alt pairs, because they collide with AltGr on international layouts. ⇧⌘S, because it is one slip from ⌘S, the sidebar toggle.
- **Existing custom keys**: someone who rebound Stop keeps their key. Everyone else moves from ⌘. to ⇧⌘X; the release notes say so.

**`resume-session`** is titled "Resume focused session" and described as "Resume or restart the stopped chat, terminal or mission slot in focus." It has scope `Global` and `fixed: false`. It needs a `ResumeFocusedSession` action next to `StopFocusedSession` in `main.rs`, and a branch in the binding build (`keymap.rs:939`). The Stop description becomes "Stop the chat, terminal or mission slot in focus."

**What "the focused session" means** is the same for both keys:

- **Chat route**: `active_focused_session_id()`, which covers the focused chat or terminal pane in the active tab. Drawer shells are out of scope.
- **Mission view**: the slot shown by the active mission tab, `MissionTab::Session(id)`. On the Feed tab, or with no slot open, both keys do nothing.
- **Any other route**: nothing.

**Stop**:

- **Chat route**: unchanged. `stop_focused_session` stops the session if it is `Running`, through `stop_chat`, including any confirmation `stop_chat` already shows.
- **Mission view**: a `StopFocusedSession` handler on `MissionWorkspace`, registered next to the `MissionTabPrevious` handler (`mission_workspace/view.rs:275`). It calls `act_on_slot(id, SessionControlKind::Stop, …)`, the path of the rail's Stop control. Guards already in `act_on_slot` (mission lifecycle busy, slot action in flight, secondary window) make it a no-op in the same cases.
- **Unchanged**: the mission-level Stop (stop every slot) keeps its button and gets no key.

**Resume**:

- **Chat route**: the session must not be `Running`. It resumes through `resume_chat(pane_id, session_id, …)` for the focused pane, the path of the pane's own Resume button. For a terminal pane that path restarts the shell, as the ended overlay's Restart does.
- **Mission view**: a `ResumeFocusedSession` handler on `MissionWorkspace` calls `act_on_slot(id, SessionControlKind::Resume, …)`, the path of the rail's Resume control and the ended overlay's Resume slot.
- **Does nothing**: while the session is running, starting, resuming or archiving; in a secondary window that does not own the session; or while a transition is in flight. These are the checks `resume_chats` and `act_on_slot` already make.

**Showing the bindings**:

- **Pane action menu** (`pane_action_items_for`, `surfaces/panes.rs:2835`): the first item follows the session. While it runs, the item is "Stop" with the stop binding. Once stopped or crashed, it is "Resume" with the resume binding, or "Restart" for a terminal pane, and choosing it takes the resume path above. Today the item is a disabled "Stop".
- **Rail slot controls**: the Stop and Resume controls on the mission rail (`mission_workspace/rail.rs`, `SessionControl`) put the binding in their tooltip, for example "Stop · ⇧⌘X", through the existing `title` tooltip.
- **Formatting**: both read their binding through `keymap::effective_binding`, so a rebound or unassigned key shows correctly.

## Non-goals

- Other shortcut changes. The hidden system shortcuts (Quit, Hide, Hide others, Minimize, Full screen, Paste) stay hidden and unchanged; Copy only joins them.
- A key for the mission-level Stop all / Resume, or for Restart slot.
- Terminal drawer shells.
- A single key that toggles between stop and resume.

## Implementation phases

One mission, one PR:

1. **Tab row and fixed rows**: the nine entries marked fixed and rendered as one row, fixed rows that turn off but take no other key, Copy moved to the reserved list, and search across the one card.
2. **Keys and routing**: the new Stop default, the `resume-session` entry and action, the mission-view handlers for both keys, and resume on the chat route.
3. **Menus and tooltips**: the pane menu item that follows the session, and the rail tooltips.

## Verification

- **Unit tests** (`keymap.rs`, `settings_page` or `surfaces` tests, `panes.rs` menu tests):
  - `stop-session` defaults to ⇧⌘X and `resume-session` to ⇧⌘R, mapped to Ctrl+Shift+X and Ctrl+Shift+R under `windows_default`, and neither conflicts with any entry or reserved shortcut.
  - The nine `select-tab-N` entries are fixed, and `effective_binding` returns ⌘1–⌘9 (Ctrl+1–Ctrl+9 under `windows_default`) even with a custom key saved.
  - A fixed entry saved as unassigned has no binding and counts as a change; the tab row is off only when all nine are; a turned-off fixed key still conflicts for another entry.
  - The shortcuts pane renders one tab row with the chip "⌘1–⌘9", not nine rows, and one card in registry order whose fixed rows are exactly New window, Go to tab 1–9 and Close pane; Copy has no row, and `find_conflict` still refuses ⌘C for a rebindable entry.
  - Search for "tab 3", "⌘3" and "go to tab" each returns the one row.
  - `pane_action_items_for` gives Stop for a running session, Resume for a stopped agent and Restart for a stopped shell, each with its binding. Update the existing `["Stop", "Rename…", …]` assertions.
- **Checks**: `runner-app` tests and workspace clippy.
- **Manual pass** (Jason, macOS and Windows):
  - The shortcuts page is one card whose three fixed rows match the design; ⌘1–⌘9 (Ctrl+1–9) still switch tabs, and ⌘C still copies a terminal selection.
  - After turning off Close pane, New window and the tab row, ⌘W, ⇧⌘N and ⌘1–⌘9 do nothing in Runner, the macOS menu bar included, and restore or Reset all brings them back.
  - Stop and resume work from the keyboard in turn on a focused chat, a terminal pane, and a mission slot open in the mission view.
  - The pane menu and the rail tooltips show both bindings.
