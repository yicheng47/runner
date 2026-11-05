# 1184 — Pane header: chat menu at the title, stop beside the status, maximize toggle

Tracking issue: [#1184](https://github.com/yicheng47/runner/issues/1184). Design: `runner/desktop/specs/1184-pane-header-rename.pen` in `yeelinks_design`, signed off by Jason on 2026-11-05: the "Final · pane header" board, the sidebar "today vs proposed" board and the maximize board. The design is authoritative on layout and spacing; this spec is authoritative on behavior.

## Motivation

The same chat has two menus that disagree. The pane header's `⋯` (`pane_action_items_for`) lists Stop/Resume ⇧⌘X, Fork chat and Rename…; its first item repeats the lifecycle button right beside it. The sidebar row menu (`tab_menu_entries`) lists Pin, Rename tab, Fork chat and Archive ⌘W, in another order and with another rename label. Rename sits in `⋯` or behind a double-click on the title that nothing hints at. Split tabs have no way to give one pane the whole tab for a while.

## Behavior

### Pane header

Left to right: runtime mark, title with a chevron, account chip (when available); then, right-aligned, the status, its lifecycle control, a divider, split, maximize (split tabs only), ×, and the side-panel toggle where it shows today (the top-right pane), after its divider.

`✳ fix: clear selection after copy ⌄ (Personal) ······ ● Working ■ | ▯ ⤢ ✕ | ▯`

- **`⋯` is removed.**
- **Title menu.** The title and its chevron form one button. A single click opens the chat menu: Rename… · Fork chat · Pin (Unpin when the tab is pinned). Archive is not listed because × beside it already archives. Fork keeps today's enabled/disabled rules and tooltip; items keep today's capability gating (`client.supports`) and the disabled state for a secondary (remote-controlled) pane. A terminal pane's menu holds Rename… and Pin. Double-clicking the title still starts rename directly. Rename edits in place as today: Enter saves, Esc cancels.
- **Status and lifecycle control.** The status (dot and label) moves from the identity group to the right, and the stop/resume control sits right after it. Everything to the right of the control has a fixed width, so a longer label grows leftward and the control keeps its position. Stopped shows `○ Stopped ▶` in the same slot. During Starting and Resuming, which have no lifecycle action, the slot keeps its width (empty) so the status does not jump when the button appears. The control's tooltip, shortcut and confirm behavior are unchanged.
- **Terminal panes** show no status and no lifecycle control, as today they have no Stop.
- **Narrow panes.** The title truncates first, then the account chip hides, then the status label hides (the dot stays); the lifecycle control and pane controls stay visible. This keeps today's priority with the new order.

### Maximize pane

- A toggle next to split, shown only when the tab has more than one pane. ⤢ (`maximize-2`) maximizes the pane: it fills the tab; the other panes keep running, hidden, and are not resized to zero for their PTYs (keep their last size). The same slot then shows ⤡ (`minimize-2`) with the selected-button background, and restores the layout exactly as it was, sizes included.
- Shortcut: new keymap entry `toggle-maximize-pane`, default ⇧⌘↩ / Ctrl+Shift+Enter, acting on the focused pane, listed in Settings → Keyboard shortcuts and the guide's shortcut table. Pick another default only if it conflicts with an existing binding, and say so in the PR.
- Maximize is transient view state in the app, keyed by tab, and is not written to the persisted `PaneLayout` (`runner-core`): relaunch, reconnect and window restore show the split. No protocol or storage change.
- Leaving maximize: the toggle, the shortcut, or anything that targets another pane of the same tab (sidebar selection, ⌘[ / ⌘], attention jumps) restores the split and then focuses that pane. Splitting the maximized pane restores the split first, then splits. Closing or archiving the maximized pane leaves the remaining layout unmaximized. A tab reduced to one pane drops the state.

### Sidebar row menu

One ordered list in both menus: Rename… · Fork chat · Pin/Unpin, a separator, then the destructive end action.

- Chat row (single chat): Rename… · Fork chat · Pin · — · Archive ⌘W.
- Split tab row: Rename tab… · Pin · — · Archive all (it archives every pane, as today).
- Terminal row: Rename… · Pin · — · Close terminal ⌘W.

Actions, shortcuts and confirmation behavior are unchanged; only labels, order and the separator change.

### Decided defaults

- The sidebar menu does not gain Stop/Resume.
- A maximized pane is not restored after relaunch.

## Non-goals

Mission workspace slot headers, the side panel, the stopped-chat card, the launcher page, remote protocol or daemon changes, and any change to what Stop, Resume, Fork, Pin, Rename or Archive do.

## Affected guide pages

`docs/guide/guides/chats-and-windows.md` (pane header, menu, split and navigate, sidebar menus), `docs/guide/getting-started/first-chat.md` (rename and the Stop chat icon's position), `docs/guide/reference/keyboard-shortcuts.md` (maximize), each mirrored in `docs/guide/zh-CN/`.

## Verification

Automated: the header's action lists (`pane_action_items_for` and its replacement), the title menu's items per runtime and capability, `tab_menu_entries` order and labels for chat, split and terminal rows, the keymap entry and its default, and the maximize state machine (toggle, restore on navigation, split, close, single pane).

Live checks in the development app on macOS (qa):

1. Header layout matches `pane-header-final.png`: no `⋯`; title chevron; status and stop at the right.
2. Title menu: click opens Rename… · Fork chat · Pin; each works; double-click on the title still renames; a terminal pane's menu holds Rename… and Pin.
3. Stop control position: a chat moving through Working → Idle → a longer label (for example Approval needed) keeps the button in place; Stop then Resume uses the same slot; starting a new chat does not shift the status when the button appears.
4. Narrow split pane: the title truncates first; status dot, stop and pane controls stay visible.
5. Maximize: in a split tab ⤢ maximizes; a busy neighbour (a terminal running `sleep 5; echo done`) keeps running and shows its output after restore; ⤡ and ⇧⌘↩ restore the same sizes; selecting the hidden pane in the sidebar restores and focuses it; a single-pane tab has no maximize button.
6. Sidebar menus: chat, split-tab and terminal rows show the order and labels above, and their actions still work.
