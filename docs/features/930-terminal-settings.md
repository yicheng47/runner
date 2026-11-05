# 930 — Terminal settings

Tracking issue: [#930](https://github.com/yicheng47/runner/issues/930). Design: `runner/desktop/specs/930-terminal-settings.pen` in the design repo, signed off by Jason on 2026-11-05 (the drawn Blink cursor toggle is on; its default is off, per Behaviour): frame `N0w45y` (option A, chosen) for Settings → Terminal and frame `WGEvV` for Settings → Appearance after the palettes move. Frame `osRyB` (option B, a live preview in the Terminal pane) was not chosen and is not built. Where the design and this spec disagree on visual detail, the design wins; on behaviour, this spec wins.

## Motivation

Settings → Terminal has three rows: font, size (10–20) and cursor shape. Everything else about the terminal is a constant, while people coming from iTerm2, Ghostty or Terminal.app expect the basic options those terminals have, and long agent transcripts run past the fixed 10,000-line scrollback. The terminal palettes live in Appearance, away from the rest of the terminal's settings.

## Scope

### Terminal pane

The pane header keeps "Terminal" with the caption "Text, colours, cursor, keyboard and scrollback for the embedded terminal." The rows are grouped into sections, each a section label above its own card, in this order:

| Section | Row | Control | Values |
| --- | --- | --- | --- |
| Text | Font family | Today's searchable font select | Unchanged |
| Text | Font size | Stepper with the range shown beside it | 8–32 px (was 10–20), default unchanged |
| Text | Line height | Stepper with the range shown beside it | 1.0–1.6 in steps of 0.1, default 1.4 |
| Colors | Light mode palette | Today's light terminal palette select | Moved from Appearance |
| Colors | Dark mode palette | Today's dark terminal palette select | Moved from Appearance |
| Cursor | Cursor style | Today's select | Unchanged |
| Cursor | Blink cursor | Toggle | Default off |
| Keyboard | Option as Meta key | Today's select (#929) | Unchanged; macOS only |
| Scrollback & selection | Scrollback | Select of presets | 1,000 · 5,000 · 10,000 (default) · 25,000 · 50,000 lines |
| Scrollback & selection | Copy on select | Toggle | Default off |

The Keyboard section shows on macOS only; Windows and Linux hide the whole section, label included, until another row lands there. #1177's "Ctrl keys in terminals" setting stays in Settings → Keyboard shortcuts.

Subtitles, as drawn: Line height "Row spacing as a multiple of the font size."; Light mode palette "Terminal colours when the app is light."; Dark mode palette "Terminal colours when the app is dark."; Blink cursor "Programs that set their own cursor still can."; Scrollback "Lines each terminal keeps above the screen. More lines use more memory in every open session."; Copy on select "Copy text to the clipboard as soon as you finish selecting it." The other rows keep today's subtitles.

### Behaviour

- **Font size.** The stepper and the stored value clamp to 8–32. Saved values inside the old 10–20 range are unchanged.
- **Line height.** Replaces the `LINE_HEIGHT_FACTOR` constant everywhere a terminal row height is computed: the terminal element, the PTY row count sent on resize, and every surface that sizes a terminal or its overlays from the font size (chat, terminal tabs, mission slots and drawers, attachments, agent-update and remote-workspace terminals). Changing it re-lays out open terminals, and the shell's row count follows, as a font-size change does today. Stored values are clamped to 1.0–1.6 and rounded to one decimal.
- **Blink cursor.** Runner did not blink the cursor before this feature, so the default stays off, a steady cursor (Jason, 2026-11-05). On sets the default cursor's blinking for every terminal; a program that sets its own cursor style through DECSCUSR still wins, both ways. Only the focused terminal in the active window blinks, at 530 ms per phase, on one timer that runs only while the setting is on and that terminal is focused and painted; unfocused and background terminals draw a steady cursor and keep no timer. A keypress, typed or pasted text, new output or a cursor move shows the cursor again and restarts the cycle, so typing keeps it visible even before the echo arrives. New and changed settings apply to every open terminal, including the agent-update dialog's.
- **Scrollback.** Replaces `TERMINAL_SCROLLBACK_LINES` and goes through the existing `TerminalMirror::configure` path, which already forwards the history size to runnerd. Changing it applies to open terminals; lowering it drops the oldest lines beyond the new limit. A stored value that is not one of the presets is clamped to 1,000–50,000. 50,000 is the cap (Jason, 2026-11-05): a cell is about 24 bytes, so at 120 columns a full session holds about 58 MB at 10,000 lines and 290 MB at 50,000, once in the app and once in runnerd.
- **Copy on select.** When on, finishing a mouse drag selection in a terminal copies its text to the clipboard, the same text ⌘C / Ctrl+C would copy; the selection stays visible. A click without a drag, or a selection made by double- or triple-click, copies when the click completes. Off (the default) keeps today's behaviour.
- **Palettes.** Only the place you choose them moves. The stored `light_terminal_theme` and `dark_terminal_theme` settings, their options and their effect are unchanged, so no migration is needed.
- **Settings search.** The Settings search finds the new rows, and finds the palettes under Terminal rather than Appearance: besides pane names, it matches the row labels of the Terminal and Appearance panes, with Option as Meta key on macOS only.
- **Remote terminals.** Line height, cursor blink and copy on select are rendering and input choices, so a terminal on another computer follows the viewer's settings, as font and cursor style already do. Its palette stays the host's, as today.

### Appearance pane

Appearance keeps Theme, Window material (macOS), the side-by-side theme preview and the macOS transparency note. The preview is unchanged: it still renders each mode's app palette with the terminal palette chosen in Terminal → Colors. The two per-mode cards become one "App palette" section with a Light mode row and a Dark mode row, each keeping today's app palette select and subtitle. The pane caption reads "Theme and the app palette for each mode; terminal palettes are in Terminal."

### Out of scope

- A live preview in the Terminal pane (option B).
- Moving "Open file links in": it stayed in General, since the #634 file viewer used the same editor setting; #634 m7 later removed the row, leaving the viewer's Open in control as the only place to choose the editor (Jason, 2026-11-05).
- Free-form scrollback entry, unlimited scrollback or a memory readout.
- Any change to palettes themselves, or to the Keyboard shortcuts pane.

## Implementation notes

- Settings live in `crates/runner-app/src/app_settings.rs`; new fields need serde defaults so existing settings files load unchanged, with normalization tests like the font size's.
- The pane is `render_terminal_settings` and the Appearance pane `render_appearance_settings` with `palette_section` in `crates/runner-app/src/surfaces/settings/page/appearance.rs`.
- Cursor blinking was hard-coded at `blinking: true` in `TerminalMirror::configure` (`crates/runner-terminal/src/terminal.rs`) and in the daemon-side model (`crates/runner-terminal/src/terminal/model.rs`), and no renderer read it. The mirror now takes the setting, and the terminal element reads the term's cursor style to blink; the daemon model and the client protocol are unchanged, since a reattach snapshot replays only a program-set cursor style.
- Copy on select hooks the terminal element's mouse-up handling in `crates/runner-app/src/terminal/element.rs`, reusing the existing copy path.

## Affected docs

- `docs/guide/guides/appearance.md`: "Theme and palettes" (terminal palettes are now chosen in Settings → Terminal → Colors) and "Size and terminal text" (the new rows, the 8–32 range, and the Keyboard section on macOS).
- `docs/guide/guides/remote-access.md`: the sentence on which terminal settings a remote terminal follows.
- The `docs/guide/zh-CN/` copies of both pages, checked with `python3 script/check-guide-translations.py`.
- `crates/runner-app/i18n/en.toml` and `zh-hans.toml` for every new or changed string.

## Verification

Headless tests in `runner-app`:

- the Terminal pane renders the five sections in order with the rows above; the Keyboard section is absent on Windows and Linux builds;
- the Appearance pane has one App palette section and no terminal palette rows;
- settings round-trip and normalize: font size 8–32, line height 1.0–1.6 rounded to 0.1, scrollback presets and clamping, blink and copy-on-select defaults (both off), and an old settings file loads with the new defaults;
- the row-height computation follows the line-height setting, including the PTY row count;
- copy on select copies on mouse-up after a drag when on and does not when off;
- a blinking cursor toggles on one timer while its terminal is painted, shows again on output or input, draws steadily and cancels its timer when not blinking, and stops its timer when the terminal is no longer painted;
- the scrollback select follows a change made in another window, and a saved non-preset value stays choosable;
- an open agent-update terminal follows a terminal-settings change.

Live checks on macOS in the development app are listed in the mission brief.
