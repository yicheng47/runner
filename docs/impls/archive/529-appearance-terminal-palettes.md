# 529 — Appearance owns app and terminal palettes per mode, with a preview

Tracking issue: [#529](https://github.com/yicheng47/runner/issues/529), phase 4, decided by Jason on 2026-09-11 during the smoke test of PR [#563](https://github.com/yicheng47/runner/pull/563). Shipped 2026-09-11 in the same PR (mission `01M282RJNRN4MQ0V1JHT98KJ1Y`, claude crew, archived by Jason mid-PR-mode; the scope change and follow-ups landed inline). Design: `design/runner.pen` `Settings — Appearance` (`k1WS7`), `Settings — Terminal` (`n5LJj`), committed `c16f1e6`; signed off. Branch: **`feat/529-runner-light-theme` already exists and is checked out** — it carries the whole 529 work and this brief; work on it, do not create another. Do not open or edit `design/runner.pen`.

> Changed after handoff (2026-09-11, Jason): the Runner Light *terminal* palette is gone and Runner Dark is plain **Runner** (key `runner`). Terminal lists are Rosé Pine Dawn / Runner / Catppuccin Mocha (light) and Runner / Catppuccin Mocha / Rosé Pine Dawn (dark). The crew shipped the four-palette version in `e32263b`; the reduction landed inline after the mission was archived.

## What ships

Match app goes away. The terminal palette is picked per mode, like the app palette already is: a light pick and a dark pick, both living in Settings → Appearance next to the app picks, and the terminal follows whichever mode is resolved. Appearance gains a live preview of both modes. Settings → Terminal keeps font, size, cursor, scrollback and loses its Theme row.

## The frame (`k1WS7`), top to bottom

1. `PaneHeader` "Appearance" as today. Card 1 holds only the **Theme** row (System / Light / Dark segmented, unchanged). The canvas shows an App font row under it; that row has no code counterpart (Inter is pinned in `app_settings.rs:58`), leave it out.
2. **Preview**: a row of two panes, `gap_4`, each `flex_1`. A pane is a 172 px tall rounded box (`rounded(12)`, 1 px `line` border, `clip`) drawn entirely from one variant's `ThemeColors`: a 92 px sidebar strip in `sidebar` with a `fg_2` brand bar, a `sidebar_selected` row, three `fg_3` bars; a main area in `bg` with a `fg` title bar and an `accent` pill; then a terminal sample box in the terminal palette's `background`, `rounded(8)`, six mono 11 px lines coloured from the palette: `foreground`, `ansi[8]`, `ansi[2]`, `ansi[3]`, `ansi[1]`, `ansi[4]`, text `❯ cargo test -p runner-backend` / `   Compiling runner-backend v0.8.6` / `test result: ok. 587 passed; 0 failed` / `warning: unused variable \`slot\`` / `error[E0308]: mismatched types` / `~/repos | Fable 5 · xhigh | US SJC`. Under each pane a `fg_2` 12 px caption: `Light · <app pick> + <terminal pick>` and `Dark · …`. The pane whose mode is currently resolved (`theme::active_variant().is_light()`) gets a 2 px `accent` border and ` · active` appended to its caption. The left pane always renders the light picks, the right the dark picks, whatever the app is showing now — so every colour in the preview comes from `theme::colors_for(variant)` and the picked `TerminalPalette`, never from the `theme::bg()`-style globals.
3. A "Light" label (`fg_2`, 13 px, medium) then a card with two rows: **App palette** (subtitle "Chrome colours when the app is light.", the existing light select) and **Terminal palette** ("Terminal colours when the app is light.", new select). Then "Dark" and the same two rows for dark.
4. All five selects the same width, text left, chevron right; swatches as today (Runner Light `0x00a66a`, Rosé Pine Dawn `0xd7827e`, Runner Dark and Carbon `0x00ff9c`, Mocha `0xcba6f7`, Latte `0x8839ef`).

## Where the code is

- `crates/runner-app/src/app_settings.rs`: `TerminalTheme` `:34` (`RunnerLight, RunnerDark, RosePineDawn, CatppuccinMocha, #[default] #[serde(alias = "runner", other)] MatchApp`) with `palette_for(self, variant)` `:45`; `AppSettings` fields `app_theme` / `light_app_theme` / `dark_app_theme` `:172-174` and `terminal_theme`; tests `terminal_theme_palettes_follow_the_variant`, `legacy_theme_settings_migrate_without_resetting_other_preferences`, `runner_terminal_themes_round_trip`, `unknown_terminal_theme_loads_as_match_app_without_resetting_settings` (`:611`).
- `crates/runner-app/src/theme.rs`: `LightTheme` / `DarkTheme` are the pattern to copy (two enums, `#[default]` + `#[serde(other)]`, `resolve_variant`), `ThemeVariant::is_light`, `colors_for(variant)`.
- `crates/runner-app/src/app_store.rs:165` `TerminalSettingsSnapshot::for_variant` carries `theme` and a `light` bool for Match app; test `:837`.
- `crates/runner-app/src/surfaces/app_shell.rs:747` `sync_theme`, `:767` `apply_terminal_palette` (the one place that pushes the palette to live terminals), `:724` the spawn-time palette.
- `crates/runner-app/src/surfaces/settings_page.rs`: selects built at `:255-290` (`light_theme`, `dark_theme`, `terminal_theme`), stored `:194`; `SettingsSelection` arms `:800-805`; `render_appearance_settings` `:1750`, `render_theme_segmented` `:1770`; `render_terminal_settings` `:1845` refreshes the terminal select options and renders the Theme row at `:1900`; option/value/parse helpers `:1982-2040`; tests from `:2085` (`theme_select_options_round_trip`, the settings visual tests).
- `crates/runner-app/src/surfaces/chat.rs` and `mission_workspace.rs:798` call `palette_for(theme::active_variant())`.
- `crates/runner-app/src/ui/settings.rs`: `SettingsCard` `:66`, `SettingsRow` `:99` with `subtitle`.
- Settings nav order is already `APP_PANES` (`settings_page.rs:114`); the canvas was aligned to it, nothing to change.

## Fix shape

1. **Model.** Replace `TerminalTheme` with `LightTerminalTheme { #[default] #[serde(other)] RosePineDawn, RunnerLight, RunnerDark, CatppuccinMocha }` and `DarkTerminalTheme { #[default] #[serde(alias = "runner", other)] RunnerDark, CatppuccinMocha, RosePineDawn, RunnerLight }`, each with `palette(self) -> TerminalPalette`. `AppSettings` gets `light_terminal_theme` (JSON `lightTerminalTheme`) and `dark_terminal_theme` with `#[serde(alias = "terminalTheme")]` so an old file's single key becomes the dark pick and writes back as `darkTerminalTheme`; `terminal_theme` is removed. A free function `terminal_palette(settings, variant) -> TerminalPalette` picks by `variant.is_light()`; `apply_terminal_palette`, the spawn sites and `TerminalSettingsSnapshot` use it, and the snapshot carries the resolved palette identity (both picks plus `is_light`) so a mode flip restyles.
2. **Selects.** Two new `StyledSelect`s (`light_terminal_theme`, `dark_terminal_theme`) with fixed option lists (no per-variant refresh; delete the refresh in `render_terminal_settings`), `SettingsSelection::{LightTerminalTheme, DarkTerminalTheme}` writing the fields; the old `TerminalTheme` selection and helpers go. Keys: `runner-light`, `runner-dark`, `rose-pine-dawn`, `catppuccin-mocha`. Give the five palette selects one width (a `SETTINGS_SELECT_WIDTH` const in `settings_page.rs`, 176 px in rems).
3. **Appearance pane** per the frame: `render_appearance_settings` becomes Theme card, preview, Light label + card, Dark label + card. The preview is a private `render_theme_preview(&self, cx)` in `settings_page.rs` (or a small `surfaces/settings/theme_preview.rs` if it passes 150 lines) taking `(ThemeColors, TerminalPalette, caption, active: bool)` per pane; colours only from its arguments. Debug selectors `SETTINGS_THEME_PREVIEW_LIGHT` / `_DARK` on the panes.
4. **Terminal pane**: drop the Theme row and the `terminal_theme` select.
5. **Docs**: spec `docs/features/529-runner-light-theme.md` "Terminal" section rewritten to the per-mode model (Match app gone, the two keys, the alias); `README.md:150` themes line to match.

## Rules of the road

- Palette constants, `ThemeVariant`, `LightTheme`/`DarkTheme`, Carbon: unchanged. No new colour literals outside `theme.rs` and the palettes; the preview's sample text is the only literal content.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app -p runner-terminal`, `make clippy` (also `--features updater`), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push (PR #563 is already open on it; do not open another), drive CI green (`gh pr checks 563 --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents. Stage by path; leave the other session's untracked docs alone.

## Tests

- `app_settings.rs`: `{"terminalTheme":"runner"}` → dark Runner Dark, light Dawn, and the next save writes `darkTerminalTheme` + `lightTerminalTheme` and no `terminalTheme`; `{"terminalTheme":"catppuccin-mocha"}` → dark Mocha; `{"terminalTheme":"match-app"}`, `"monokai"` and a missing key → defaults; both new keys round-trip all four values; `terminal_palette` picks the light field under Runner Light and Latte, the dark field under Carbon and Mocha.
- `app_store.rs`: the snapshot changes when either pick changes and when the variant flips modes; unchanged when the variant flips within a mode (Carbon → Mocha).
- `settings_page.rs`: the two option lists in order with swatches; `VisualTestContext`: the preview's light pane shows the light app `bg` and the picked light terminal `background` while the active variant is Carbon (proves it does not read the globals), the active outline follows the resolved mode, and the Terminal pane has no `SETTINGS_TERMINAL_THEME` row; existing settings tests updated.

## Jason's smoke test (after landing)

1. Appearance → Theme = Dark: the dark pane is outlined; pick Catppuccin Mocha as the dark terminal palette: the preview's dark terminal sample and every open terminal restyle at once.
2. Theme = Light: the light pane is outlined, terminals switch to the light pick without a restart; pick Runner Light there and back to Rosé Pine Dawn.
3. Theme = System, flip macOS appearance: outline and terminals follow.
4. A `ui-settings.json` saved by 0.8.6 with `"terminalTheme":"catppuccin-mocha"` opens with Mocha as the dark pick and Dawn as the light pick; after any change the file has both new keys.
5. Settings → Terminal shows font, size, cursor, scrollback only.

## Non-goals

Custom palettes, importing themes, per-surface overrides, the Windows chrome, Claude Code or Codex theme settings.
