# Command palette on ⌘⇧P

Tracking issue: [#559](https://github.com/yicheng47/runner/issues/559). Status: planned, design first. Priority P2.

## Motivation

Runner has a palette on ⌘K (`crates/runner-app/src/surfaces/command_palette.rs`), but it is a switcher: it lists missions, chats, runners, and crews, plus two fixed rows, New terminal and Settings. There is no way to run a command from the keyboard without already knowing its shortcut, and the shortcuts are only discoverable by opening Settings → Keyboard shortcuts. Jason asked for a VS Code-style command palette on ⌘⇧P on 2026-09-11.

The keymap in `crates/runner-app/src/keymap.rs` already holds every action with an id, a title, a description, a `KeymapScope`, and the user's current binding, and each action is a GPUI action the shortcut dispatches. A command palette is mostly a new view over that data plus the handful of actions that have no shortcut today.

## Behavior

### Two entry points, one surface

- **⌘⇧P** (Ctrl+Shift+P on Windows through the existing `windows_default` mapping) opens the palette overlay in **command mode**.
- **⌘K keeps the switcher** exactly as it is. Typing `>` as the first character of the ⌘K query flips the same overlay into command mode; deleting the `>` flips it back. ⌘⇧P is the same overlay opened with the query pre-set to `>`, cursor after it, so the two entry points are one state machine, not two palettes.
- The overlay, input, keyboard handling (↑ ↓ Enter Escape, hover sets active), focus restore on dismiss, and the `deferred` z-order in `app_shell.rs` are all reused; command mode changes what `items` holds and how a row renders.

### What a command is

- Every `KeymapEntry` from `keymap::entries()` is a command: title, description, scope, and the live `KeyCombo` after user rebindings. The palette reads the same resolved bindings Settings → Keyboard shortcuts shows, so a rebound shortcut is shown rebound.
- Actions that have no shortcut today join the list as commands with an empty pill: Stop all sessions in this tab, Restart session, Rename chat, Archive chat, Fork chat, the layout-picker presets, Start mission, Check for updates, and one row per Settings page (General, Appearance, Chat, Terminal, Agents, Skills, Keyboard shortcuts, Missions, Archived, Diagnostics, About).
- System rows stay out: Quit, Hide, Hide others, Minimize, Close window, Toggle fullscreen, Paste. They are menu items and the palette would only duplicate the menu.
- The switcher's two fixed rows, New terminal and Settings, are commands too; they keep appearing in ⌘K so nothing regresses.

### Row

Title on the left in the body weight, the description as a muted second line, the shortcut pill on the right using the formatter the keyboard-shortcuts page already uses (⌘⇧P, ⌥F12, Ctrl+Shift+P on Windows). Rows without a shortcut have no pill. The row icon is the scope's icon (a small global, split, mission, or terminal glyph) so scope is legible at a glance without a label.

### Selecting a row

Selecting dispatches the same GPUI action the shortcut dispatches (`window.dispatch_action`) to the element that had focus before the palette opened, then dismisses. No second implementation per command: if the action is wired for the keybinding it works from the palette, and a command that is disabled in the current context does the same nothing the shortcut would.

### Scope gating

Follows `KeymapScope`: Global commands are always listed; ChatSplit commands only while the focused surface is a split chat tab; Mission commands only inside a mission workspace; Terminal commands only while a terminal has focus. The scope is decided from the focus captured at open, the same focus the dispatch targets.

### Ordering and groups

- Empty query: a **Recent** group of the last five commands run from the palette, then the rest grouped by scope in the order Global, then the focused surface's scope, each group under a small uppercase header in the switcher's existing header style. Within a group, keymap order.
- Recents persist in `ui-settings.json` as `paletteRecentCommands` (a list of command ids, serde default empty); ids that no longer resolve are dropped on read.
- Non-empty query: one flat list, no headers, ordered Recent first then keymap order.

### Matching

Case-insensitive. Every whitespace-separated query word must appear as a substring of the command's title, description, or id words, in any order, so `split down` and `down split` both find Split pane down. No fuzzy scoring, no ranking beyond the group order above. The switcher's matching is unchanged.

### Keymap entries

- The existing `command-palette` entry (⌘K) is retitled **Quick switcher** with the description "Search missions, chats, runners, and crews." Its id does not change, so a saved rebinding survives.
- A new `commands` entry titled **Command palette**, description "Run any command by name.", scope Global, default ⌘⇧P, rebindable. ⌘⇧P is unbound today and `shift-cmd-p` is not a terminal key, so no conflict row appears in Settings.

## Non-goals

- Chords, argument-taking commands (a command that then prompts for input), and a `?` help mode.
- Fuzzy matching or learned ranking beyond the five recents.
- New actions. Every palette row is an action the app already has; a command that needs new behaviour is its own feature.
- Palette theming beyond the existing overlay tokens.
- A menu bar mirror of the command list.

## Design

`design/runner.pen`: one frame, the palette overlay in command mode over the 2-pane chat, showing the `>` prefix in the input, a Recent group, the Global and Chat split groups with headers, shortcut pills on the right, and one shortcut-less row. Pencil-first per the post-cutover rule; code does not start until Jason has signed the frame off.

## Implementation Phases

1. **Design.** The one frame above. Stop for sign-off.
2. **Registry and mode.** A `Command` list built from `keymap::entries()` plus the shortcut-less actions; the `>` prefix and the pre-set query; the `commands` keymap entry and the retitled `command-palette` entry; dispatch to the previous focus; scope gating from the captured focus; recents persisted with a serde default.
3. **Rows.** Shortcut pills through the existing binding formatter, description line, group headers; Windows verified on `nightly-windows` with Ctrl+Shift+P.

## Verification

- ⌘⇧P opens the palette with `>` in the input; Escape restores the previous focus; ⌘K still opens the switcher unchanged, and typing `>` there shows the same command list.
- Every non-system keymap entry appears in command mode with the same pill Settings → Keyboard shortcuts shows; rebinding one in Settings changes its pill.
- Selecting Split pane right from the palette does what ⌘D does in the same tab; selecting a ChatSplit command is impossible from a single-pane tab because the row is not listed.
- Running three commands puts them under Recent on the next open, in most-recent-first order, and they survive a relaunch; an old `ui-settings.json` without `paletteRecentCommands` loads with an empty Recent group and is not rewritten.
- `split down` and `down split` both match Split pane down; `zoom` lists the three zoom rows in keymap order.
- `make verify` green.
