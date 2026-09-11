# 529 — Runner Light: theme, terminal palette, literal audit

Tracking issue: [#529](https://github.com/yicheng47/runner/issues/529). Spec: [529](../../features/archive/529-runner-light-theme.md), phases 2 and 3 (phase 1, the design, is signed off: Jason 2026-09-11 on `qBQHS`). Feature, P1. Shipped 2026-09-11 in [#563](https://github.com/yicheng47/runner/pull/563) (mission `01M27SJTCE4Q3BW73HWRGHVN9N`, codex-crew). Branch: **`feat/529-runner-light-theme` already exists and is checked out** — it carries the design commit and this brief; work on it, do not create another. Do not open or edit `design/runner.pen`; every value you need is in this brief.

## After the mission (2026-09-11, Jason's smoke test)

Four commits landed on the branch after the crew's PR opened: the Resuming pill moved to the accent family; Claude Code is spawned with `"theme": "auto"` in the `--settings` JSON Runner already sends, so it follows the terminal's background answer instead of the user's pinned theme; Rosé Pine Dawn joined the terminal list as a third-party light palette (canvas frame `VLI02`); Monokai left the list for its licence, a stored `monokai` loading as Match app. The lists below describe the code at launch.

## What ships

`ThemeVariant::RunnerLight` as the default light theme, read off the canvas; Codex Light removed; a `TerminalTheme` list of Match app (default), Runner Light, Runner Dark, Catppuccin Mocha, Monokai, with Match app following the resolved app variant; the colour literals outside `theme.rs` moved onto tokens; snapshot tests pinning Carbon and Runner Light.

## Token values (the canvas `mode: light` axis, final)

`RUNNER_LIGHT: ThemeColors` in `crates/runner-app/src/theme.rs`, field by field:

| field | value | | field | value |
|---|---|---|---|---|
| `bg` | `0xf6f6f8` | | `fg` | `0x1c1d22` |
| `panel` | `0xffffff` | | `fg_2` | `0x5f616b` |
| `raised` | `0xecedf1` | | `fg_3` | `0x9a9ca6` |
| `line` | `0xe4e5ea` | | `accent` | `0x00a66a` |
| `line_strong` | `0xd6d8df` | | `accent_ink` | `0xffffff` |
| `sidebar` | `0xeeeff3` | | `warn` | `0xc27c0e` |
| `sidebar_selected` | `0xe1e3e9` | | `danger` | `0xd63b57` |
| `sidebar_selected_border` | `0xd3d6de` | | `info` | `0x0a8fb3` |

Appearance swatch for the select option: the accent, `0x00a66a`.

## Terminal palette (canvas frame `Runner Light · terminal palette`, final)

`palette::RUNNER_LIGHT` in `crates/runner-terminal/src/palette.rs`, same shape as `RUNNER` (`:20`): background `0xf6f6f8` (the app `bg`, so pane and chat are one surface as in Carbon; the canvas frame shows `#fafafb` and the spec's "ground is the app bg" wins), foreground `0x1c1d22`, cursor `0x00a66a`, selection `0xd6e9df`. ANSI 0–7: `5f616b d63b57 00a66a c27c0e 2563eb 7c3aed 0a8fb3 b4b7c1`. ANSI 8–15: `6e717c e0526e 0bbf80 d9932a 3b82f6 a78bfa 22a6c9 c9cbd3`. The four achromatic slots were revised after Jason’s 2026-09-11 smoke test: foreground < black < bright black < white < bright white < background, keeping all four darker than the light ground.

## Where the code is

- `crates/runner-app/src/theme.rs`: `LightTheme` `:76` (`Codex` is `#[default]`, `CatppuccinLatte`), `DarkTheme` `:84`, `ThemeVariant` `:92`, `resolve_variant` `:100` (`LightTheme::Codex => ThemeVariant::Codex` at `:113`), `ThemeColors` `:125`, tables `CARBON` `:144`, `CATPPUCCIN_MOCHA` `:163`, `CODEX` `:182`, `CATPPUCCIN_LATTE` `:201`, `ACTIVE_VARIANT` as `AtomicU8` `:220` with `set_active_variant` / `active_variant` (a `u8` match, `:226`) / `colors_for` `:236`, tests from `:325` (`resolves_auto_and_explicit_intents` uses `LightTheme::Codex` at `:340` and `:349`).
- `crates/runner-app/src/app_settings.rs:34`: `TerminalTheme { CatppuccinMocha, Monokai, #[default] #[serde(other)] Runner }` with `palette(self) -> TerminalPalette` at `:43`. `light_app_theme: LightTheme` (JSON `lightAppTheme`, camelCase throughout; the default at `:210` is `LightTheme::Codex` and the test at `:530` asserts it) is the Appearance light pick.
- `crates/runner-app/src/surfaces/app_shell.rs:738` `sync_theme` resolves and stores the variant on every appearance change; `:716` and `crates/runner-app/src/surfaces/mission_workspace.rs:798` build terminal styles from `settings.terminal_theme.palette()`; `crates/runner-app/src/surfaces/settings_page.rs:859` pushes `set_palette` to live terminals when the Terminal pane's select changes.
- `crates/runner-app/src/app_store.rs:165` `TerminalSettingsSnapshot::from(&AppSettings)` copies `terminal_theme`; `:202` hashes its discriminant to decide whether terminals restyle.
- `crates/runner-app/src/surfaces/settings_page.rs`: Appearance light select options `:266-267` (`codex`, `catppuccin-latte`), Terminal theme options `:288-290` (`runner`, `catppuccin-mocha`, `monokai`), `light_theme_value` / `parse_light_theme` `:1982-1990`, `terminal_theme_value` / `parse_terminal_theme` `:2010-2022`, the `SettingsSelection::TerminalTheme` write at `:811`.
- Colour literals outside `theme.rs` named by the spec: `surfaces/app_shell.rs:644` (`hsla(0,0,0,0.35)` scrim), `surfaces/panes.rs:1681` and `:2041`, `surfaces/mission_workspace.rs:3909`, `:4482`, `:5388` (all `to_hsla(palette.background …)` / selection), `platform_ui/windows.rs:229` (`0xc42b1c` close hover) and `:242` (`0xffffff`).

## Fix shape

1. **Theme tables.** Add `ThemeVariant::RunnerLight` and `RUNNER_LIGHT`; `LightTheme::RunnerLight` becomes `#[default]` and `Codex` is removed along with `ThemeVariant::Codex` and the `CODEX` table. Give `LightTheme` `#[serde(other)]` on the default so a stored `"codex"` loads as Runner Light (write a test with the literal JSON). `resolve_variant`, `active_variant`, `colors_for` follow. Appearance light options: Runner Light (`runner-light`), Catppuccin Latte; `light_theme_value` / `parse_light_theme` accordingly.
2. **Terminal themes.** `TerminalTheme { MatchApp (#[default], key `match-app`, serde alias `runner`), RunnerLight (`runner-light`), RunnerDark (`runner-dark`), CatppuccinMocha, Monokai }`. Replace `palette(self)` with `palette_for(self, variant: ThemeVariant)`: Match app gives `RUNNER_LIGHT` when `variant.is_light()` (Runner Light, Latte) and `RUNNER` otherwise; the rest are fixed. Every caller passes `theme::active_variant()`. `TerminalSettingsSnapshot` carries the resolved palette's discriminating input (theme plus the resolved light/dark) so a Match app terminal restyles when `sync_theme` flips the variant: after `set_active_variant` in `sync_theme`, push `set_palette` to live terminals the way `settings_page.rs:859` does, factoring that push into one helper both call. Terminal select options in this order and with these labels: Match app, Runner Light, Runner Dark, Catppuccin Mocha, Monokai; swatches `0x00a66a` for the two Runner entries, Match app the accent of the active variant.
3. **Literal audit.** The six `to_hsla(palette…)` sites are already palette-driven and are correct once the palette follows the app; leave them but confirm each reads the *resolved* palette. `app_shell.rs:644` scrim becomes a theme helper (`theme::scrim()`: black at 0.35 in dark variants, black at 0.2 in light). `platform_ui/windows.rs:229` and `:242` become a pair on the theme (`theme::window_close_hover()` / `window_close_hover_ink()`), dark values unchanged, light `0xc42b1c` / `0xffffff` as Windows 11 does in both modes; no other Windows chrome change.
4. **Snapshot pins.** Two `VisualTestContext` tests: sidebar plus mission workspace, and a confirm dialog, each rendered once under Carbon and once under Runner Light at one window size, asserting the `bg`, `panel`, `sidebar` and accent fills read back through `debug_bounds`/fills equal the table values (not pixel screenshots). Existing theme tests updated for the enum change.

## Rules of the road

- Carbon does not change; Mocha, Latte, Monokai tables unchanged.
- No per-surface overrides, no theme editor. One variant everywhere.
- Windows-only code compiles on macOS only under `cfg(windows)`; keep the Windows change minimal and do not claim it was run.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app -p runner-terminal`, `make clippy` (also with `--features updater`), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `theme.rs`: `resolve_variant` covers Auto/Light/Dark with Runner Light as the default light pick; `"codex"` in a `LightTheme` JSON deserialises to `RunnerLight`; `RUNNER_LIGHT` equals the table above field by field (a drift guard).
- `app_settings.rs`: `"runner"` and a missing key load as `MatchApp`; `match-app` / `runner-light` / `runner-dark` round-trip; `palette_for(MatchApp, RunnerLight) == RUNNER_LIGHT`, `palette_for(MatchApp, Carbon) == RUNNER`.
- `palette.rs`: `RUNNER_LIGHT` background equals the app light `bg` and the 16 ANSI entries match the list above.
- `settings_page.rs`: the two selects list exactly the options above in order.
- The two snapshot pins from step 4.

## Jason's smoke test (after landing)

1. Fresh profile, Appearance → Light: the whole app is Runner Light, terminals in a claude-code and a codex chat are light with the same ground as the chat around them.
2. Settings → Terminal → Theme reads Match app, Runner Light, Runner Dark, Catppuccin Mocha, Monokai; pick Runner Dark under the light app: dark terminal in a light app, survives restart.
3. Appearance → Auto, flip macOS appearance while a mission runs: app and terminals flip together; walk sidebar, split chat, feed, settings, Start Chat, confirm dialog, command palette for dark fragments.
4. A `ui-settings.json` with `"lightAppTheme": "codex"` loads as Runner Light; one with `catppuccin-latte` stays Latte.

## Non-goals

The dark theme, per-surface overrides, a theme editor, a Latte terminal palette, Windows header redesign.
