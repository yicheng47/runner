# #930 — Terminal settings

Jason requested this mission with the claude trio on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-930-terminal-settings` on `feat/930-terminal-settings`, based on `origin/main` at `1695238837376550ebae2cd23429fbae2524b2da`. The root checkout stays on main.

## Problem and outcome

Issue [#930](https://github.com/yicheng47/runner/issues/930): Settings → Terminal offers only font, size (10–20) and cursor shape; scrollback, cursor blink and line height are constants, there is no copy on select, and the terminal palettes sit in Appearance. The outcome is a Terminal pane grouped into Text, Colors, Cursor, Keyboard and Scrollback & selection, with the new rows working in every terminal, and an Appearance pane that keeps only the app palettes beside its unchanged preview.

The spec is [`docs/features/930-terminal-settings.md`](../../features/930-terminal-settings.md); it is authoritative on behaviour. The design is `runner/desktop/specs/930-terminal-settings.pen` in the design repo (`~/repos/yeelinks/yeelinks_design`, pull first): frame `N0w45y` (option A, chosen) for Terminal and `WGEvV` for Appearance. It is authoritative on visual detail. Frame `osRyB` (option B) is not built. Decisions signed off by Jason on 2026-11-05: option A, scrollback capped at 50,000, palettes move to Terminal.

## Read first

- `crates/runner-app/src/app_settings.rs`: `TERMINAL_FONT_SIZE_MIN`/`MAX`, `TERMINAL_SCROLLBACK_LINES`, the terminal fields, their normalization and tests, and where `terminal.configure(...)` is called.
- `crates/runner-app/src/surfaces/settings/page/appearance.rs`: `render_terminal_settings`, `render_appearance_settings`, `palette_section`, `render_theme_preview`; `page/mod.rs` for the settings page state and search.
- `crates/runner-terminal/src/terminal.rs` `TerminalMirror::configure` and `terminal/model.rs`: the hard-coded `blinking: true` and the scrollback forwarded to runnerd.
- `crates/runner-app/src/terminal/element.rs`: `LINE_HEIGHT_FACTOR` and the mouse-up selection handling; every other `LINE_HEIGHT_FACTOR` use (`terminal/control.rs`, `surfaces/agent_update.rs`, `surfaces/remote_client/workspace/render.rs`, `surfaces/chat/attachment.rs`, `surfaces/mission_workspace/attach.rs`) and the resize row computation.

## Deliverables

1. The settings, their normalization and defaults, with old settings files loading unchanged, as the spec lists.
2. The Terminal pane with its five sections and rows; the Keyboard section on macOS only.
3. Line height, cursor blink, scrollback and copy on select applied to every terminal surface, live when changed.
4. Appearance with one App palette section and the unchanged preview; Settings search finding the moved and new rows.
5. Strings in `en.toml` and `zh-hans.toml`; the guide pages the spec names, with their zh-CN copies passing `python3 script/check-guide-translations.py`.

Prefer no client-protocol change; the spec explains why blink should stay app-side. If one is unavoidable, say so in the PR, which then takes `core-review` and loses `frontend-only`.

## Validation

Automated: the headless tests the spec lists, `runner-app` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`, `git diff --check` and the guide translation check, with this worktree's own target directory. CI green on macOS, Windows and Linux.

Live checks for qa in the development app on macOS, each with its reason. Back up the development settings file before starting and restore it at the end.

1. Settings → Terminal matches frame `N0w45y`: sections in order, rows, controls and subtitles; the font-size stepper stops at 8 and 32 (new pane and range).
2. In a terminal tab, line height 1.0 and then 1.6: rows tighten and spread with no clipped or overlapping rows, and `tput lines` changes with it; repeat once in a chat pane (row height and PTY rows follow the setting).
3. Blink cursor off: the prompt caret is steady; on: it blinks (new toggle).
4. Scrollback 1,000, then `seq 1 3000`: scrolling to the top shows no line below about 2,000; at 50,000 a new `seq 1 20000` keeps line 1 (history size applied live).
5. Copy on select on: drag-select text in a terminal and paste it into the launcher's input; off: the clipboard is unchanged after a drag (new behaviour).
6. Terminal → Colors changes an open terminal's palette live, and Settings → Appearance matches frame `WGEvV`, with its preview showing the palette just chosen (palette move).
7. Set non-default values for every new row, quit and relaunch the development app: the values persist (new settings fields).

## Authorization and handoff

Jason approved the claude trio (coder, reviewer, qa) for this mission. The crew may commit, push this branch and open one PR against main, labelled `frontend-only` if no protocol change was needed, a risk label picked from the diff, and `needs-live-test` until qa passes every check above. Stop at an open PR with green CI, reviewed and live-tested, and report the PR URL, final SHA, changes and validation through Runner.
