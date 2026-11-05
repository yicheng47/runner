# 1177 — Ctrl keys in terminals on Windows and Linux

Tracking issue: [#1177](https://github.com/yicheng47/runner/issues/1177). Design: `runner/desktop/specs/1177-terminal-shortcuts.pen` in the design repo, frame `ss4SN` (Windows / Linux, shown with Terminal first on) and frame `u47PjU` (macOS), signed off by Jason on 2026-11-05. Where the design and this spec disagree on visual detail, the design wins; on behaviour, this spec wins.

## Motivation

On Windows and Linux, `windows_default` turns each macOS ⌘ shortcut into the same Ctrl chord. GPUI matches key bindings before a focused terminal's key handler runs, so Ctrl+D (end of input), Ctrl+K (delete to end of line), Ctrl+S (forward history search) and Ctrl+M (Return) never reach the shell or agent: they split a pane, open the command palette, toggle the sidebar and minimize the window instead. The approach follows Orca: Runner's shortcuts keep winning by default, the defaults that break everyday terminal use move off bare Ctrl, and a setting lets terminals have every Ctrl key.

## Scope

### New Windows and Linux defaults

| Action | Today | New default |
| --- | --- | --- |
| Split pane right | Ctrl+D | Ctrl+Shift+D |
| Split pane down | Ctrl+Shift+D | Alt+Shift+D |
| Command palette | Ctrl+K | Ctrl+Shift+P |
| Toggle sidebar | Ctrl+S | Ctrl+B |

- Every other default keeps its key, including new tab on Ctrl+N, Ctrl+[ / Ctrl+] for panes and mission tabs, Ctrl+1–9, Ctrl+, and zoom. macOS defaults are unchanged.
- A user's saved override of any of these shortcuts keeps working; only defaults move. Conflict detection treats the new defaults like any other binding.
- No new default uses Ctrl+Alt, which is AltGr text input on Windows and many Linux layouts.

### Minimize has no shortcut on Windows and Linux

On Windows and Linux, Minimize has no shortcut, so Ctrl+M reaches the terminal; the `system-minimize` keymap entry exists only on macOS. macOS keeps ⌘M for Minimize, with **Window → Minimize** showing it as the key equivalent, as before (Jason, 2026-11-05, after QA). The window's minimize button stays on every platform. The macOS **Window** menu drops its **Maximize** item, which duplicated the system's Fill and Zoom (Jason, 2026-11-05).

### Ctrl keys in terminals (Windows and Linux)

A setting, titled **Ctrl keys in terminals**, with a segmented control:

- **Runner first** (default): Runner's shortcuts work everywhere, including while a terminal has focus.
- **Terminal first**: while a terminal has focus, a Runner shortcut whose chord is Ctrl plus one key, without Shift or Alt, does not fire, so the key reaches the terminal. With the defaults this frees Ctrl+N, Ctrl+B, Ctrl+[ / Ctrl+], Ctrl+1–9 and Ctrl+,. The rule applies to the effective binding, so a user override onto a bare Ctrl chord is freed too.
- Terminal first does not affect copy with a selection, paste, or zoom. Outside a terminal, every shortcut works in both modes.
- "A terminal has focus" means the `Terminal` key context: chat panes, terminal tabs, mission slots, the mission drawer and remote terminals.
- The setting is stored with the other app settings (`app_settings.rs`), takes effect immediately, and does not exist on macOS, where ⌘ never reaches a terminal.

### Settings → Keyboard shortcuts

- The page is grouped into sections on every platform, in this order: **Navigation and creation**, **Panes and sessions**, **Missions**, **Terminal**, **Zoom and window**, each a labelled card as the design draws. Search filters rows within their sections and hides a section with no match.
- On Windows and Linux, **Ctrl keys in terminals** is the first row of the Terminal section. While Terminal first is on, every row whose effective binding it frees shows an **Outside terminals** tag before its key chip.
- The page's caption, search, Reset all and per-row edit, restore and unbind controls are unchanged.

### Out of scope

- Any macOS shortcut change.
- Ctrl+Shift+C / Ctrl+Shift+V as extra copy and paste keys.
- The Ctrl+C copy-or-interrupt rule, fixed separately in #1178.

## Affected docs

- `docs/guide/reference/keyboard-shortcuts.md` and its zh-CN copy: the new defaults, minimize removed on Windows and Linux, the setting, and the page's sections.
- `docs/guide/guides/chats-and-windows.md` and `docs/guide/getting-started/first-chat.md`, with their zh-CN copies: the split, command palette and sidebar keys.
- `crates/runner-app/i18n/en.toml` and `zh-hans.toml`: section titles, the setting's title, description and options, and the tag.
- `docs/tests/regression/`: any case naming Ctrl+D, Ctrl+K, Ctrl+S or minimize as a Windows key.

## Verification

In-process tests in `runner-app`:

- the Windows/Linux defaults for split right and down, command palette and sidebar, and macOS unchanged;
- ⌘M minimizes on macOS, and Windows and Linux have no minimize binding or keymap entry;
- Runner first: a focused terminal's bare Ctrl shortcut still fires its action;
- Terminal first: with a terminal focused, Ctrl+N, Ctrl+B, Ctrl+[ and Ctrl+1 reach the terminal and fire nothing, while Ctrl+Shift+P, Ctrl+Shift+D, Ctrl+V and zoom still fire; outside a terminal the bare Ctrl shortcuts fire;
- a user override onto a bare Ctrl chord follows the policy;
- the setting persists and round-trips;
- the settings page renders the five sections, the setting row only on Windows/Linux, and the tags only while Terminal first is on.

Live checks, on Windows, in the development app:

1. In a terminal tab at a shell prompt, Ctrl+K deletes to the end of the line and Ctrl+D at an empty prompt ends the shell, with no palette or split.
2. Ctrl+Shift+D splits right, Alt+Shift+D splits down, Ctrl+Shift+P opens the command palette and Ctrl+B toggles the sidebar, from a terminal and from the sidebar.
3. Settings → Keyboard shortcuts shows the five sections, the setting as the Terminal section's first row, and the tags once Terminal first is on.
4. With Terminal first on, Ctrl+N at a shell prompt reaches the shell (next history entry) and opens no tab; from the sidebar it still opens a new tab. Back on Runner first, Ctrl+N in the terminal opens a tab again.

On macOS, one live check: Settings → Keyboard shortcuts shows the five sections with no setting row, ⌘M minimizes the window, **Window → Minimize** works and the **Window** menu has no **Maximize**.
