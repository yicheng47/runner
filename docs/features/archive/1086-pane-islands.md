# 1086 — Pane islands, and terminals without a lifecycle

Historical design recovery (2026-11-12): superseded desktop spec canvases referenced in this record are preserved in `YeelinksCo/yeelinks_design` at revision `4605d1e43ab6835a555fdd3957469a6b55ddaec8`, under `runner/desktop/specs/` (including `archive/`). Their original frame ids resolve there; current product designs are in `runner/desktop/runner.pen`.

> Tracking issue: [#1086](https://github.com/yicheng47/runner/issues/1086), which absorbed [#900](https://github.com/yicheng47/runner/issues/900) (closed 2026-10-30).
> Status: shipped in 0.19.0 on 2026-10-31, through [#1087](https://github.com/yicheng47/runner/pull/1087) and [#1091](https://github.com/yicheng47/runner/pull/1091); the grip repair for [#1093](https://github.com/yicheng47/runner/issues/1093) landed with #1092's launcher in [#1094](https://github.com/yicheng47/runner/pull/1094). Design signed off by Jason on 2026-10-30. Validation: [macOS QA record](../../tests/archive/1086-pane-islands.md).
> Design: `runner/desktop/specs/1086-pane-islands.pen` in `yeelinks_design` (mission board at `0c7df46`; light palette at `c716cbf`; scrollbar board “Sidebar scrollbar · B” at `29aa9ca`; lifecycle board “Island header · lifecycle control” at `2588dac`, with the button's placement set by “Island header · lifecycle button pinned right” (Jason, 2026-10-31); stopped-chat board at `7a0f20b`); the #634 screens in `runner/desktop/specs/634-file-preview.pen` are drawn in the same islands.
> Related: [634](634-file-preview.md) (the side panel's file preview), [#567](https://github.com/yicheng47/runner/issues/567) (pane × archives).

## Motivation

A chat tab is one work card: a 44 px tab bar over its panes and the side panel, split by 1 px rules. A split tab adds a 26 px header to each pane and dims the unfocused ones to 70%. Tab-wide controls (Stop all, Resume all, fork, the drawer and side panel toggles) live in the tab bar, away from the pane they act on, and a chat tab also has a bottom terminal drawer (⌥F12), a second way to hold a shell beside the pane tree.

Drawing each pane as its own island gives every session a clear edge and one header for its name, status and controls, for any split. The drawer then has no reason to exist: a shell is just another pane.

Shells also carry a lifecycle they do not need. Stopping a shell leaves a dimmed pane with a "Shell exited · exit 1" card and two different Restart controls, and the "exit 1" is really the SIGHUP Stop sent. Stop exists for agents because their conversation can be resumed. A terminal such as Ghostty has no stop: a shell ends by closing its pane or exiting, and Ctrl+C handles a stuck command.

## Scope

- App scrollbars follow option B from `sidebar-scrollbar-b.png` (board “Sidebar scrollbar · B”, design revision `29aa9ca`): hidden at rest, including their click targets; shown while scrolling, dragging or hovering their parent list. A visible thumb is 4 px wide, with its right edge 3 px from the edge, in muted at 60%; hovering the 10 px hit strip or dragging widens it to 6 px leftward. It hides 900 ms after the last scroll or after leaving the list (plain hide). Keyboard input does not end pointer hover. Terminal scrollbars and reserved app-scrollbar gutters stay unchanged.

### Islands

- Every pane of a chat or terminal tab is its own island: a card in the workspace background (`theme::bg`) with a 12 px radius and a 1 px border, 6 px apart on the window material, with the same 8 px outer inset the work card has today. An expanded sidebar has a 6 px gap before the first island; a collapsed sidebar keeps the 8 px left inset. The focused island's border brightens (white at about 20% in dark, ink at about 22% in light); unfocused panes keep `UNFOCUSED_PANE_OPACITY`. The gaps after the expanded sidebar, between panes and before the side panel are the resize gutters, with the grab strip covering each whole gap: hovering or dragging them shows only the resize cursor, with no accent bar.
- There is no tab bar. Its controls move into each island's 40 px header.
- The #634 side panel is the last island in the row, unchanged inside; its own header (Info, Files, collapse) is the island header.
- A mission tab has a mission island (existing header, member tabs, feed and composer) beside a full-height Sessions/Info rail island, whose own header is the island header. An open terminal drawer is a third island below the mission, with a 6 px row-resize gutter between them. All use the same 12 px radius, 1 px border, 6 px sidebar/panel gaps and 8 px outer insets as chat tabs. Every gutter’s strip covers its whole gap, with only the resize cursor and no line or accent highlight. The drawer header is 40 px tall, with 10 px side padding, no bottom rule, and the existing 24 px shell tabs, + and chevron-down hide control. The drawer island height keeps the saved 280 px default and 120–600 px limits, without migration; its terminal-size estimate accounts for the new header, border and corner clearance. Its border brightens while its shell has focus and the mission border returns to normal; existing mission dimming, duplicate-window and remote/take-back behavior remain. The mission header’s drawer toggle stays in place. Role, crew, Settings and the other routes keep the work card.

### Island header

- **Left:** in a tab with two or more panes, a 14 px six-dot grip (`grip-vertical`) sits 4 px before the runtime mark (shell mark for a terminal), then the name, Claude account chip when present, and status. Split headers have an 8 px left inset; single-pane headers have no grip and keep a 14 px inset. The grip stays in narrow panes, using faint at rest, muted on header hover, text under the pointer, and an open-hand cursor. It is the pane drag source; the runtime mark is not. macOS main windows set `app_owns_titlebar_drag` so AppKit leaves top-row grip presses to Runner; the app’s existing header drag and double-click behavior handles the remaining titlebar areas. Design follow-up #1093: board “Island header · pane drag grip (#1093)” in `runner/desktop/specs/1086-pane-islands.pen`, revision `c7c5861`, exported as `pane-drag-grip.png`.
- **Right:** a chat’s lifecycle button and its divider, then the menu (…), the split icon (Split Right ⌘D, Split Down ⇧⌘D) and ×, on every island.
- **Side panel toggle:** the island whose top-right corner is the tab area's top-right corner ends its header with the toggle, after a 1 px divider that separates it from that pane's own buttons. It stays where the tab bar's right end puts it today, whatever the split. Hidden while the panel is open (the panel's collapse button replaces it) and while a shell is focused, as today.
- **Status:** the island header draws every state’s label at 12 px (`theme::text_ui()`), medium weight, with a 6 px glyph-label gap, keeping today’s glyphs and colours: ring for Idle; spinner for Working, Starting and Resuming; square for Stopped; the warning colour when the agent needs you; danger for errors; the status reports state only and never changes on hover or focus. Island headers use available space: hide detail first, then truncate the name with an ellipsis down to about 32 px, then hide the status label in favor of its glyph. The divider, lifecycle button and right-side actions keep their space. A 360 px stopped Claude pane with an account chip keeps the Stopped label; a 240 px pane keeps the glyph and Resume button. Other status surfaces retain their width thresholds. Visible detail keeps its bounded width and truncates inside the header.
- **A chat’s lifecycle control is an always-visible icon button that leads the right-hand actions.** A 22 px button with a 6 px radius and a 13 px icon sits 8 px after the identity; a 1 px divider (14 px tall, `border_strong`) follows with 4 px to the button and 4 px to the menu, as the side panel toggle’s divider has. Its distance from the island’s right edge never depends on the status, so it holds still while the status gains or loses a detail (Working · Using tools, Idle · Interrupted, Idle). The nightly placed it after the status, whose slot is 180 px with a detail and label-wide without one, so the button jumped by up to about 140 px during a turn. While the session runs it shows circle-stop in muted, whatever the agent is doing; once stopped it shows circle-play in accent. Hover and focus tint Stop with danger (12% fill, 35% border, danger icon), and Resume with accent (10% fill, 40% border). The tooltip names the action and effective shortcut: Stop chat ⇧⌘X or Resume chat ⇧⌘R by default on macOS. Shells, Starting/Resuming, a disabled lifecycle and secondary views show neither divider nor button, following the current `pane_lifecycle_action` rules and remote capability filter. Narrow panes keep the button. Stop ends the process and keeps the chat. The menu is unchanged. `island-header-lifecycle-button.png` supersedes the hover rows of `island-header-states.png`.
- **Stopped chat card:** centered in the pane over the existing 70% dim, 360 px wide or the pane width minus 32 px. Use panel fill, a 1 px border, 12 px radius, 24 px top/side and 20 px bottom padding, centered content with 10 px gaps: a 36 px raised round badge with a 15 px muted square; **Chat stopped** in semibold `text_lead`; the unchanged reason in 13 px muted text with 1.5 line height; and 4 px extra space before one Primary Resume button with play icon and the effective resume-session shortcut at 55% opacity. The card never exceeds the pane body minus 16 px at the top and bottom. When height is insufficient, drop the badge, clamp the reason to two lines with an ellipsis, then one line, then omit the reason; retain the title and Resume, omitting the title only if those cannot fit together. Resume remains reachable. At very narrow widths its shortcut can stack under its label, and padding contracts only as needed to fit the button. Caller-supplied titles remain; Archive leaves this card because × archives. Mission member Resume slot/Restart slot cards stay unchanged. `stopped-chat-card.png` at `7a0f20b` supersedes the earlier stopped-card and narrow-header behavior.
- With a control on every chat island, Stop all and Resume all go away. ⇧⌘X and ⇧⌘R keep acting on the focused chat.
- **×** archives a chat through today's confirmation ("…is still running. Archiving stops it…") and closes a terminal; ⌘W does the same on the focused pane.

### Menu (…)

- A chat: **Stop** (danger, ⇧⌘X) or **Resume** (accent, ⇧⌘R) first, then **Fork chat**, then **Rename…**. Fork keeps `header_fork_state`'s rules: hidden for shells and secondary views, disabled without a reason when the runtime cannot fork, and with "No session key captured yet" when it has no key.
- **Archive chat** leaves the menu, because × archives.
- A terminal: **Rename…** only.

### Terminals have no lifecycle

- No Stop, no stopped state and no Restart for shells: in the island header, the menu, ⇧⌘X, ⇧⌘R and Stop all. The terminal status reports activity only and is never a button.
- A shell ends with × or ⌘W, or by exiting; its pane closes on any exit, clean or not. The shell-exited overlay (`SessionOverlay::shell_exited`) goes away for every shell.
- Shells left stopped by a daemon crash close their panes the same way.
- Launch resume is unchanged: after a planned stop (an update, or quitting under a policy that resumes), terminal panes come back as today.
- This applies to every shell: tab panes and mission workspace shells.

### No drawer in chat tabs

- A chat tab has no terminal drawer. A shell is an ordinary pane, split from a chat or terminal pane.
- ⌥F12 (Toggle terminal drawer) is retired in chat and terminal tabs.
- On upgrade, the shells of a chat tab's saved drawer are closed and the drawer leaves the tab's layout.
- Mission tabs keep their drawer.

### Light palette E

Runner Light uses the following signed-off surface tokens. Raised surfaces, text and the other app palettes keep their current colors; Glass derives its material color from the new sidebar token.

| Token | Value |
| --- | --- |
| sidebar / window material | `0xe2e4e9` |
| bg | `0xf2f3f5` |
| panel | `0xf8f8fa` |
| line | `0xd9dbe1` |
| sidebar_selected | `0xd6d9e0` |
| sidebar_selected_border | `0xcaced7` |
| line_strong | `0xcdd0d7` |

Rosé Pine Dawn’s terminal background and cursor accent use Runner’s `0xf5f1eb` ground, instead of upstream’s `0xfaf4ed`; its remaining colors are unchanged. The reference is `split-chat-tab-light.png` from design revision `c716cbf`.

### Fixed opening window minimum

Every platform passes a fixed minimum when opening a window: `(saved sidebar width + 6 + MIN_SPLIT_PANE_WIDTH + 8) × zoom` wide, and `max(400, 8 + 40 + MIN_SPLIT_PANE_HEIGHT + 8) × zoom` high. The sidebar’s fixed content with the computer picker measured 364.5 px at 1× in an in-process layout fixture; 400 px leaves headroom. This supersedes the dynamic window-minimum proposal: later zoom/sidebar changes do not update an open window’s floor, and splits retain today’s squeeze behavior. There are no native update hooks, mission minimum plumbing or split-child constraints. Jason waived live QA for this follow-up.

## Implementation phases

1. **Terminals have no lifecycle.** Independent of the islands, and can land first.
2. **No drawer in chat tabs,** including the upgrade that closes saved drawer shells and the remote companion's drawer requests for chat tabs.
3. **Islands and the island header,** on macOS, Windows and Linux, including the #634 side panel.
4. **Mission islands,** with a separate full-height Sessions/Info rail and, when open, a drawer island below the mission.
5. **Light palette E,** including the Rosé Pine Dawn ground override.
6. **App scrollbars shown while in use,** across every app list and field.
7. **A fixed opening window minimum,** shared by all platforms.

## Verification

- Unit tests for the pane menu items (chat running, chat stopped, no session key, terminal), the island header’s always-visible lifecycle button by status, its divider and action/shortcut tooltip, its fixed position while the status loses its detail, the centered stopped-chat card and effective shortcut, responsive headers at 360/240 px, the side panel toggle's island, the removed ⌥F12 binding, the drawer upgrade and the close-on-exit rule for shells, including after a crash, and launch resume of terminal panes.
- Live checks in the development app: a split chat tab with a chat, a working chat and a shell; Stop chat and Resume chat from the header icon button; × on a running chat; the side panel toggle across splits; the #634 panel open; a terminal tab whose shells exit cleanly and with a non-zero code; a mission workspace shell; a chat tab saved with a drawer on the previous build, after the upgrade.
- Mission-island follow-up: rendered checks for sidebar/rail/drawer gaps across zoom and collapsed states, 40 px drawer header, focus borders, terminal-size estimate, take-back coverage and existing header/feed alignment; QA compares dark/light mission and drawer PNGs at 100%/130%, including closed-to-open and focus transitions; this runs together with phase 5. New gutter drags are left to Jason.
- Palette E: pinned Runner Light and terminal palette tests, Glass color derivation, and QA’s comparison to the light split-chat PNG plus a light chat, terminal, mission and one Settings/Role/Crew surface for selections, inputs, menus and borders. Run with the mission drawer follow-up in one final QA pass.
- App scrollbar tests pin rest-hidden and hidden click-through, scroll/list hover visibility, 4/6 px widths and the 900 ms linger; QA checks the sidebar, one Settings list and one select in dark/light against the option-B PNG. Thumb dragging is left to Jason.
- One small app test pins the opening minimum’s saved sidebar width and zoom. Native min-resize checks are left to Jason under his explicit QA waiver.
- Windows and Linux in CI, with native checks left to Jason.

## Affected documentation

[Appearance](../../guide/guides/appearance.md), [Missions](../../guide/concepts/missions.md), [Sessions](../../guide/concepts/sessions.md), [Chats and windows](../../guide/guides/chats-and-windows.md), [First chat](../../guide/getting-started/first-chat.md), [Remote access](../../guide/guides/remote-access.md) where it lists drawer shells, [Keyboard shortcuts](../../guide/reference/keyboard-shortcuts.md), and their zh-CN copies; [architecture §3.6](../../arch/arch.md#36-surface-hierarchy--project--window--tab--pane) and the drawer and shell paragraphs it links; the [workspace regression suite](../../tests/regression/workspace.md) rows for drawers (WS-DRAWER-01/02), shells (WS-SHELL-*) and the work card (WS-CARD-*).
