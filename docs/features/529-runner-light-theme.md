# Runner Light, a first-party light theme

Tracking issue: [#529](https://github.com/yicheng47/runner/issues/529). Status: planned, design first. Priority P1.

## Motivation

Some colleagues run Runner in light mode. What they get is Codex Light or Catppuccin Latte (`crates/runner-app/src/theme.rs:125`, `:144`), palettes carried over as data from the Tauri line's CSS tokens at the rewrite and never designed for Runner. Codex Light in particular is another product's palette wearing Runner's layout. `design/runner.pen` is dark-only, so every light rendering is a token swap on a dark layout. The terminal is worse: all three terminal palettes (`app_settings.rs:30`, Runner / Catppuccin Mocha / Monokai) are dark, so a light app wraps a dark terminal on every chat and slot.

Runner/Carbon is the first-party dark theme, designed. There is no first-party light theme. Feature [15](./archive/15-light-theme.md) added the light *mechanism* (Auto / Light / Dark intent, a light and a dark pick) but shipped borrowed palettes. This spec is the palette.

## Behavior

### Runner Light

A new theme variant, **Runner Light**, designed by Jason in Pencil and owned by the product the way Runner/Carbon is. It becomes the default `LightTheme` and **replaces Codex Light**, which is removed (decided 2026-09-11: "No need for codex light anymore"). Catppuccin Latte remains selectable. Appearance → Light theme lists Runner Light first, then Latte.

A user who never touched the light-theme setting resolves to Runner Light after the update (the serde default flips). A user who had chosen Codex Light lands on Runner Light, since that variant no longer exists: the stored `light: "codex"` deserialises to the default and `ui-settings.json` is rewritten on the next save. A user who chose Latte keeps it.

### The design is the source of truth

The 16 tokens of `ThemeColors` (`bg`, `panel`, `raised`, `line`, `line_strong`, `sidebar`, `sidebar_selected`, `sidebar_selected_border`, `fg`, `fg_2`, `fg_3`, `accent`, `accent_ink`, `warn`, `danger`, `info`) are read off the signed-off canvas, not invented in code. `design/runner.pen` already carries these as variables (`bg`, `panel`, `raised`, `line`, `line-strong`, `sidebar`, `sidebar-selected`, `sidebar-selected-border`, `text-1/2/3`, `accent`, `accent-ink`, `warn`, …); the design work is a `mode: light` theme axis on those variables so every existing frame flips to light without redrawing. The key frames are then reviewed light: sidebar, direct chat with a split, mission workspace with feed and slot terminal, settings, Start Chat, confirm dialog.

Per the post-cutover rule, this is Pencil-first, and per the sign-off rule the code phase does not start until Jason has approved the frames.

### Terminal: Match app by default, Runner Light selectable

Settings → Terminal → Theme gains the entries the `Settings — Terminal` frame already shows and the code never grew. The list reads **Match app**, **Runner Light**, **Runner Dark**, **Rosé Pine Dawn**, **Catppuccin Mocha** (Monokai removed 2026-09-11 for its licence; a stored `monokai` loads as Match app):

- **Match app** (`TerminalTheme::MatchApp`, key `match-app`, the default) renders Runner Dark under a dark app variant and, since 2026-09-11, Rosé Pine Dawn under a light one (Jason after the smoke test: "this looks much better than our previous own Runner light theme"), so a fresh install set to Light gets a light terminal without touching the Terminal pane, and Auto intent flips app and terminal together. Runner Light stays selectable as an explicit pick. Known consequence: Dawn's warm cream ground (`#FAF4ED`) sits on Runner Light's cool `#F6F6F8`, so a terminal pane reads as a faint warm rectangle against the chat until the app's light variant is warmed to match.
- **Runner Light** (`TerminalTheme::RunnerLight`, key `runner-light`) is the new palette as an explicit pick: 16 ANSI colors plus fg, bg, cursor, selection, designed on the same canvas as the app tokens (`Runner Light · terminal palette`, `W7oJm`). Its ground is the app `bg`, so a terminal pane and the chat around it are one surface, as they are in Carbon. It lives beside the other palettes in `crates/runner-terminal/src/palette.rs` as `RUNNER_LIGHT`.
- **Runner Dark** is today's `Runner` palette under its honest name, key `runner-dark`, for a dark terminal inside a light app; Mocha stays for the same purpose.
- **Rosé Pine Dawn** (`TerminalTheme::RosePineDawn`, key `rose-pine-dawn`, added 2026-09-11 at Jason's ask after the smoke test) is the official Rosé Pine Dawn terminal palette as a third-party light choice beside Runner Light, canvas frame `Rosé Pine Dawn — terminal palette (529)` (`VLI02`). Match app never picks it.

Loading an existing `ui-settings.json`: `terminal_theme: "runner"` was the only first-party choice before this spec and meant "the Runner palette", which now follows the app, so it loads as Match app; `catppuccin-mocha` and `monokai` keep their meaning; a missing key is Match app. The file is rewritten on the next save. A Catppuccin Latte terminal palette was filed separately as #528 and closed into this spec on 2026-09-09; it can ride on the same list later if anyone asks.

### No literal colors outside the theme

Eight color literals live outside `theme.rs` and the terminal element (`surfaces/mission_workspace.rs` ×3, `surfaces/panes.rs` ×2, `platform_ui/windows.rs` ×2, `surfaces/app_shell.rs` ×1). Each moves onto a token or gets a light/dark pair; none may survive as a dark-only constant. The terminal element's 17 literals are the ANSI defaults and are covered by the palette work.

## Non-goals

- A theme editor or user-defined palettes. Runner Light is one designed theme, like Carbon.
- Restyling the dark theme. Carbon does not change.
- Per-surface light overrides (a dark sidebar in a light app). One variant, applied everywhere.
- Windows chrome recolor beyond what the two `platform_ui/windows.rs` literals need; the header design ([494](./archive/494-macos-header-navigation.md), `design/windows-header.pen`) is separate.

## Design

`design/runner.pen`: the `mode` theme axis with `dark` and `light` values on the shared variables (plus new `danger` and `info` tokens), the **DS · Option 4 — Runner Light** sheet (`WFqT1`) in the slot the Codex Light option used to occupy, a **Light** band (divider at y 22500) with the main chat page (`qBQHS`) and the mission workspace (`cPsVA`) rendered in the light axis, and the **Runner Light · terminal palette** frame (`W7oJm`). Phase 1 done 2026-09-11 on `feat/529-runner-light-theme`; Jason overwrites the token values in the variables panel, and the remaining key frames (settings, Start Chat, confirm dialog, split chat) join the band as light copies when reviewed.

## Implementation Phases

1. **Design.** Light axis on the `.pen` variables; Light band with the key frames; first-pass token values; terminal palette frame. Stop for sign-off.
2. **Theme.** `ThemeVariant::RunnerLight` and `RUNNER_LIGHT: ThemeColors` from the canvas; `TerminalTheme::{MatchApp, RunnerLight, RunnerDark}` with `palette::RUNNER_LIGHT`, keys `match-app` / `runner-light` / `runner-dark`, the legacy `runner` key loading as `MatchApp`, `MatchApp` resolving through the app variant wherever the terminal palette is picked, and the Terminal pane's Theme select in the order Match app, Runner Light, Runner Dark, Catppuccin Mocha, Monokai; `LightTheme::RunnerLight` as `#[default]`; `LightTheme::Codex`, `ThemeVariant::Codex`, and the `CODEX` table removed, with `light: "codex"` in an existing `ui-settings.json` falling back to the default on load (a serde `#[serde(other)]` or an `unknown → default` deserialiser, covered by a test); Appearance pane ordering; `resolve_variant` test. The palette constant lives beside the other terminal themes.
3. **Audit and pins.** The eight literals onto tokens; `VisualTestContext` snapshots of the sidebar, mission workspace, and a confirm dialog in Carbon and Runner Light at one window size, so a regression in either variant fails a test.

## Verification

- Fresh `ui-settings.json`, Appearance set to Light: the app renders Runner Light, the Match app terminal theme shows Rosé Pine Dawn in a claude-code and a codex slot.
- Settings → Terminal → Theme reads Match app, Runner Light, Runner Dark, Rosé Pine Dawn, Catppuccin Mocha, with Match app selected on a fresh profile and on a profile whose file said `runner` or `monokai`; picking Runner Light or Rosé Pine Dawn under a dark app gives a light terminal in a dark app, picking Runner Dark or Mocha under a light app gives a dark terminal in a light app; both survive a restart.
- Existing settings with `light: "codex"`: load without error and render Runner Light; Appearance shows Runner Light selected. Settings with `light: "catppuccin-latte"` still render Latte.
- Auto intent with macOS switching appearance while a mission is running: app and terminal flip together, no dark fragments left on any surface (walk sidebar, chat split, mission feed, settings, Start Chat, confirm dialog, command palette).
- Windows build: header and title bar match the light variant.
- `make verify` green; the two snapshot tests pass on CI.
