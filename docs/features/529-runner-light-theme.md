# Runner Light, a first-party light theme

Tracking issue: [#529](https://github.com/yicheng47/runner/issues/529). Status: planned, design first. Priority P1.

## Motivation

Some colleagues run Runner in light mode. What they get is Codex Light or Catppuccin Latte (`crates/runner-app/src/theme.rs:125`, `:144`), palettes carried over as data from the Tauri line's CSS tokens at the rewrite and never designed for Runner. `design/runner.pen` is dark-only, so every light rendering is a token swap on a dark layout. The terminal is worse: all three terminal palettes (`app_settings.rs:30`, Runner / Catppuccin Mocha / Monokai) are dark, so a light app wraps a dark terminal on every chat and slot.

Runner/Carbon is the first-party dark theme, designed. There is no first-party light theme. Feature [15](./archive/15-light-theme.md) added the light *mechanism* (Auto / Light / Dark intent, a light and a dark pick) but shipped borrowed palettes. This spec is the palette.

## Behavior

### Runner Light

A new theme variant, **Runner Light**, designed by Jason in Pencil and owned by the product the way Runner/Carbon is. It becomes the default `LightTheme`; Codex Light and Catppuccin Latte remain selectable. Appearance → Light theme lists Runner Light first.

A user who never touched the light-theme setting resolves to Runner Light after the update (the serde default flips). A user who explicitly chose Codex Light or Latte keeps it; `ui-settings.json` is not rewritten.

### The design is the source of truth

The 16 tokens of `ThemeColors` (`bg`, `panel`, `raised`, `line`, `line_strong`, `sidebar`, `sidebar_selected`, `sidebar_selected_border`, `fg`, `fg_2`, `fg_3`, `accent`, `accent_ink`, `warn`, `danger`, `info`) are read off the signed-off canvas, not invented in code. `design/runner.pen` already carries these as variables (`bg`, `panel`, `raised`, `line`, `line-strong`, `sidebar`, `sidebar-selected`, `sidebar-selected-border`, `text-1/2/3`, `accent`, `accent-ink`, `warn`, …); the design work is a `mode: light` theme axis on those variables so every existing frame flips to light without redrawing. The key frames are then reviewed light: sidebar, direct chat with a split, mission workspace with feed and slot terminal, settings, Start Chat, confirm dialog.

Per the post-cutover rule, this is Pencil-first, and per the sign-off rule the code phase does not start until Jason has approved the frames.

### Terminal follows the app

The `Runner` terminal theme becomes theme-following: when the resolved app variant is light it renders the new **Runner Light terminal palette** (16 ANSI colors, fg, bg, cursor, selection), otherwise today's dark palette. Monokai and Catppuccin Mocha stay as they are, dark, for people who want a dark terminal inside a light app. The palette is designed on the same canvas as the app tokens so the slot terminal and the feed beside it agree. It lives beside the other palettes in `crates/runner-terminal/src/palette.rs`. A Catppuccin Latte terminal palette was filed separately as #528 and closed into this spec on 2026-09-09; it can ride on the same mechanism later if anyone asks.

### No literal colors outside the theme

Eight color literals live outside `theme.rs` and the terminal element (`surfaces/mission_workspace.rs` ×3, `surfaces/panes.rs` ×2, `platform_ui/windows.rs` ×2, `surfaces/app_shell.rs` ×1). Each moves onto a token or gets a light/dark pair; none may survive as a dark-only constant. The terminal element's 17 literals are the ANSI defaults and are covered by the palette work.

## Non-goals

- A theme editor or user-defined palettes. Runner Light is one designed theme, like Carbon.
- Restyling the dark theme. Carbon does not change.
- Per-surface light overrides (a dark sidebar in a light app). One variant, applied everywhere.
- Windows chrome recolor beyond what the two `platform_ui/windows.rs` literals need; the header design ([494](./494-macos-header-navigation.md), `design/windows-header.pen`) is separate.

## Design

`design/runner.pen`: add the `mode` theme axis with a `light` value on the shared variables, then a **Light** band with the six key frames above rendered in the light axis. The terminal palette gets its own frame beside the existing terminal theme swatches. Jason authors the values; the agent's job in phase 1 is the axis plumbing, the band, and a first proposal of token values for him to overwrite.

## Implementation Phases

1. **Design.** Light axis on the `.pen` variables; Light band with the key frames; first-pass token values; terminal palette frame. Stop for sign-off.
2. **Theme.** `ThemeVariant::RunnerLight` and `RUNNER_LIGHT: ThemeColors` from the canvas; `LightTheme::RunnerLight` as `#[default]`; Appearance pane ordering; `resolve_variant` test. Runner terminal theme resolves to the light palette under a light variant; the palette constant lives beside the other terminal themes.
3. **Audit and pins.** The eight literals onto tokens; `VisualTestContext` snapshots of the sidebar, mission workspace, and a confirm dialog in Carbon and Runner Light at one window size, so a regression in either variant fails a test.

## Verification

- Fresh `ui-settings.json`, Appearance set to Light: the app renders Runner Light, the Runner terminal theme shows the light palette in a claude-code and a codex slot, and the feed beside the slot reads as one surface.
- Existing settings with `light: "codex"`: still Codex Light after the update.
- Auto intent with macOS switching appearance while a mission is running: app and terminal flip together, no dark fragments left on any surface (walk sidebar, chat split, mission feed, settings, Start Chat, confirm dialog, command palette).
- Windows build: header and title bar match the light variant.
- `make verify` green; the two snapshot tests pass on CI.
