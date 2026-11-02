# 897 — Find in terminal

Tracking issue: [#897](https://github.com/yicheng47/runner/issues/897). Design: `runner/desktop/specs/897-terminal-find.pen` in `yeelinks_design` (commit `ac804f7`), signed off by Jason on 2026-11-02. The Pencil frames win on visual detail; this spec wins on behaviour.

## Motivation

Runner's terminal has no search. Ghostty, iTerm2 and Terminal.app all find text in scrollback; in Runner the only way to locate an earlier build error or test failure is to scroll for it. This is the largest everyday gap between Runner and a standalone terminal.

## Decisions

Jason signed off each recommendation on 2026-11-02; the alternatives are drawn beside them on the design's Open decisions board.

- **Placement.** The find bar floats over the terminal's top-right corner (`cmp/TerminalFindBar`). The terminal keeps its size, so opening or closing find never resizes the PTY, reflows the grid or makes an agent TUI redraw. In a pane narrower than the bar, the bar narrows to fit and its count and step controls wrap onto a second line, so the query field stays usable.
- **Case and regex.** The bar has Aa (match case) and .* (regex) toggles, both off by default, so a plain query is case-insensitive literal text.
- **Alternate screen.** Find opens on the alternate screen and searches only the visible screen, with the hint row "Alternate screen: only the visible screen is searched". The hint follows the terminal's `ALT_SCREEN` mode, not the runtime.
- **Live output.** While the bar is open, matches are recomputed as output arrives, the count updates and the current match stays on the same text. Once a match is selected the viewport holds still while output arrives. Keeping the current match exact once the scrollback limit is full may use a new read-only accessor in the vendored `alacritty_terminal`, under the accessor-only exception in `vendor/alacritty_terminal/RUNNER_PATCH.md`.
- **Shortcuts.** ⌘F, rebindable in Settings → Keyboard shortcuts. Windows and Linux default to Ctrl+Shift+F so Ctrl+F still reaches the shell, and ⌘G / ⇧⌘G become F3 / ⇧F3.
- **Esc.** Esc closes the bar only while the bar has focus. Once the terminal has focus, Esc goes to the process, because agent TUIs use it to interrupt.

## Behaviour

**Open.** ⌘F opens the bar on the focused terminal and focuses its field: chat panes, mission slots, and shell or drawer terminals alike. A terminal selection fills the query; ⌘F with the bar already open selects the query. Each terminal view has its own bar and keeps its query while the bar stays open. Find state belongs to the view, not the session, because one session can be shown in more than one place.

**Search.** The search runs in the app against the session's mirror `Term` (`TerminalMirror.term`), over the active grid including scrollback, with `alacritty_terminal`'s `RegexSearch` and `RegexIter`, so matches cross soft wraps. It runs as you type. `RegexSearch::new` has smart case built in, so the toggles are applied in the pattern: match case off prefixes `(?i)`, on prefixes `(?-i)`; regex off escapes the query with `regex::escape`. The grid keeps zero-width characters (combining marks, variation selectors, ZWJ) on the previous cell, where `RegexSearch` does not read them, so a plain-text query drops the characters `unicode-width` classifies as zero-width before matching: a selection or paste of a decomposed é, ❤️ or a ZWJ emoji is found, and a decomposed é also matches a plain e (Jason accepted this over-match on 2026-11-02). A regex pattern stays as typed, so marks in it cannot match. A plain-text query of only such characters behaves as an empty query. A pattern that fails to build, or that can match the empty string, shows the invalid regex state. An empty query shows no count and no highlights. The count is capped at 1,000 and then reads "1000+".

**Highlight.** Every visible match gets an amber tint and the current one solid amber with a 1 px outline and dark text. The colours are app theme tokens, not terminal palette colours, so they read over any ANSI colour: dark `#FFB020` at 25% and `#FFB020` with outline `#FFD27A` and text `#15161B`; light `#F2B544` at 35% and `#F2B544` with outline `#C27C0E` and text `#1C1D22`. The selection paints above match highlights.

**Step.** Matches are numbered from the newest output, so the bottom-most match is 1. A new query selects the newest match at or above the bottom of the viewport. Enter and ⌘G go to the next (older) match, ⇧Enter and ⇧⌘G to the previous (newer) one, wrapping at the ends. The viewport scrolls only when the match is off screen, centring it, and always through local scrolling, never a mouse report to the program. With no results, stepping is disabled and the count shows in the danger colour.

**Live output.** As above under Decisions. Matches that fall out of the scrollback limit leave the count. A resize, a clear or a switch between the primary and alternate screens recomputes the matches.

**Close.** Esc (while the bar has focus) or × closes the bar, clears the highlights, returns focus to the terminal and restores the scroll position from before find opened. Leaving the tab or mission closes find the same way: its state is discarded and the scroll position from before find opened is restored (Jason, 2026-11-02). So does switching from a mission slot to another slot or the feed; the mission drawer stays on screen across slots and keeps its find. Clicking the terminal moves focus there and leaves the bar and highlights in place.

**Keymap.** Find in terminal is a rebindable Terminal-scope entry in `keymap.rs`, listed in Settings → Keyboard shortcuts after Toggle terminal drawer. Enter, ⇧Enter, Esc, ⌘G and ⇧⌘G inside the bar are fixed.

## Not in this issue

Match marks on the scrollbar, searching several panes at once, find in the mission feed or file preview, and the terminal in the agent update dialog.

## Implementation phases

1. Search state and matching beside the terminal model, with in-process tests.
2. Match highlights in `TerminalElement`'s paint and theme tokens for both themes.
3. The find bar component, its states and the pane wiring for chat panes, mission slots and drawer terminals: open, focus, stepping, Esc and close.
4. Live output: recomputing on output and keeping the current match and the viewport in place.
5. The keymap entry, Windows/Linux defaults, the Settings row, English and Simplified Chinese strings, and the guide pages.

## Guide pages

`docs/guide/reference/keyboard-shortcuts.md` and `docs/guide/zh-CN/reference/keyboard-shortcuts.md` gain Find in terminal and the in-bar keys. A short find section goes in the guide page that covers terminal panes.

## Verification

In-process tests cover: a match across a soft wrap; the count and its cap; both toggles; literal escaping; an invalid and an empty-matching pattern; the alternate screen searching only the visible screen; stepping and wrapping; the current match staying on its text while output scrolls lines into history, before and after the scrollback limit is full; and the Windows/Linux defaults.

Live checks in the development app, left to a qa slot or Jason: open, type and step in a chat pane, a mission slot and a drawer terminal; highlights in dark and light themes; the alternate-screen hint in Claude Code; Esc in the bar versus in the terminal; the scroll position restored on close; and a streaming build with the bar open.
