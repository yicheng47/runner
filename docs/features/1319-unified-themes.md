# 1319 — One theme picker

Tracking issue: [#1319](https://github.com/yicheng47/runner/issues/1319). Design: `runner/desktop/specs/1319-unified-themes.pen` in `YeelinksCo/yeelinks_design`, revision `055fe95`, signed off 2026-11-14 (option C).

## Motivation

Appearance picks a light and a dark app theme, and Settings → Terminal → Colors picks a light and a dark terminal palette, independently. Any palette can sit under any app theme, so people have to find good pairings themselves, and some pairings, such as Catppuccin Mocha's terminal under Runner Light, are never what anyone wants. A theme should be one designed pairing of app colours and terminal palette, and Runner should offer more of them. This replaces #934, whose palette-import idea is dropped.

## Scope

### Appearance

Frames `TV1e1` (dark) and `iPu8z` (light), with the card component `cmp/ThemeCard` (`hmanH`).

- **Theme** (System, Light, Dark) and **Window material** stay as they are.
- The large two-pane theme preview and the **App palette** card are removed.
- Two sections follow: **Light theme** (“Used when the app is light”) and **Dark theme** (“Used when the app is dark”). Each is a panel holding one row of six theme cards. The header of the section for the mode in use now appends “· in use now”.
- A theme card is a thumbnail drawn in the theme's own colours: a sidebar strip with a selected row, a title bar with an accent pill, and a terminal block with a few coloured lines from the theme's terminal palette. Its name sits below. The selected card has a 2 px accent ring, its name in the primary text colour at medium weight, and a check after the name; the others have a 1 px line border and secondary text.
- Clicking a card selects that theme for its mode and applies it at once when that mode is in use: app colours and the terminal palette of every open terminal together.
- The caption reads “Pick a theme for each mode. A theme sets the app and terminal colours together.” The Reduce Transparency note stays.
- Windows and Linux use the same sections; Window material stays macOS-only, as today.

### Terminal

Frame `c4x25`. The **Colors** group and its **Light mode palette** and **Dark mode palette** rows are removed. The caption becomes “Text, cursor, keyboard and scrollback for the embedded terminal. Terminal colours come from the theme in Appearance.” The other groups are unchanged.

### Themes

| Light theme | Dark theme | Terminal palettes |
|---|---|---|
| Runner Light | Runner | Runner Light keeps today's default, `ROSE_PINE_DAWN` (Runner's muted cream ground); Runner keeps `RUNNER`. |
| Catppuccin Latte | Catppuccin Mocha | New `CATPPUCCIN_LATTE`; `CATPPUCCIN_MOCHA` unchanged. |
| Rosé Pine Dawn | Rosé Pine Moon | `ROSE_PINE_DAWN` (shared with Runner Light); new `ROSE_PINE_MOON`. |
| Tokyo Night Day | Tokyo Night | New `TOKYO_NIGHT_DAY` and `TOKYO_NIGHT`. |
| Gruvbox Light | Gruvbox Dark | New `GRUVBOX_LIGHT` and `GRUVBOX_DARK`. |
| Solarized Light | Solarized Dark | New `SOLARIZED_LIGHT` and `SOLARIZED_DARK`. |

Runner, Runner Light and both Catppuccin app themes keep their current `ThemeColors`. Each new app theme gets a full `ThemeColors` set built from its upstream palette the way the Catppuccin themes are: `bg` is the base ground, `panel`, `raised` and the `line` pair step from the theme's own surface and overlay shades, `sidebar` follows Runner's pattern (lighter than `bg` in dark themes, darker in light themes), the `fg` trio maps to text, subtle and muted text, and `accent`, `warn`, `danger` and `info` take the theme's own accent, yellow, red and cyan or blue. The reference values used for the design cards:

| Theme | bg | sidebar | selected | fg | fg_3 | accent |
|---|---|---|---|---|---|---|
| Rosé Pine Dawn | `#FAF4ED` | `#F2E9E1` | `#DFDAD9` | `#575279` | `#9893A5` | `#286983` |
| Rosé Pine Moon | `#232136` | `#2A273F` | `#393552` | `#E0DEF4` | `#6E6A86` | `#EA9A97` |
| Tokyo Night Day | `#E1E2E7` | `#D0D5E3` | `#C4C8DA` | `#3760BF` | `#848CB5` | `#2E7DE9` |
| Tokyo Night | `#1A1B26` | `#24283B` | `#2F3549` | `#C0CAF5` | `#565F89` | `#7AA2F7` |
| Gruvbox Light | `#FBF1C7` | `#EBDBB2` | `#D5C4A1` | `#3C3836` | `#928374` | `#AF3A03` |
| Gruvbox Dark | `#282828` | `#3C3836` | `#504945` | `#EBDBB2` | `#928374` | `#FE8019` |
| Solarized Light | `#FDF6E3` | `#EEE8D5` | `#E4DCC4` | `#586E75` | `#93A1A1` | `#268BD2` |
| Solarized Dark | `#002B36` | `#073642` | `#0E4552` | `#93A1A1` | `#586E75` | `#268BD2` |

Terminal palettes use the upstream ANSI 16, foreground, background, cursor and selection colours. The terminal ground may equal the app `bg`. Text in every theme must stay readable on its surfaces, including Glass chrome, the find highlight and selection.

Before bundling, check each new family's licence (Rosé Pine, Gruvbox and Solarized are expected to be MIT; Tokyo Night, from `folke/tokyonight.nvim`, Apache-2.0) and credit all six families, Runner excepted, in the app's licence notices.

### Settings and migration

- `lightAppTheme` and `darkAppTheme` stay and gain the new kebab-case values (`rose-pine-dawn`, `tokyo-night-day`, `gruvbox-light`, `solarized-light`; `rose-pine-moon`, `tokyo-night`, `gruvbox-dark`, `solarized-dark`). Unknown values still fall back to the defaults.
- `lightTerminalTheme`, `darkTerminalTheme` and the pre-0.8.7 `terminalTheme` alias are no longer read or written; an old settings file loads, keeps its app themes, and drops these keys on the next save. A custom pairing is not preserved: each mode gets its theme's palette. The release notes say so.
- The terminal palette sent to `runnerd` (`terminal_palette`) is now derived from the resolved theme variant. The protocol carries the palette by value, so no protocol change is needed, and remote terminals keep using their host's palette.

### Out of scope

Importing theme files, user-defined themes, per-chat themes, and themes for the iPhone or web remote.

## Implementation Phases

1. **Model:** extend `LightTheme`, `DarkTheme` and `ThemeVariant` with the new themes, add their `ThemeColors`, add the terminal palettes in `crates/runner-terminal/src/palette.rs`, and give each variant its terminal palette. Remove `LightTerminalTheme`, `DarkTerminalTheme`, their settings fields and selections, and derive `terminal_palette` from the resolved variant. Update the settings serialization tests for the dropped keys and the new values.
2. **Appearance:** replace the theme preview and App palette card with the two galleries and the theme card element; selecting a card saves the mode's theme and applies it. English and Chinese strings.
3. **Terminal:** remove the Colors group and update the caption.
4. **Docs:** `docs/guide/guides/appearance.md` and `docs/guide/zh-CN/guides/appearance.md` (Theme and palettes, and the Terminal group list), plus any guide page that names Settings → Terminal → Colors or the terminal palettes, such as `remote-access.md` and `chats-and-windows.md` and their Chinese pages. Licence notices for the bundled themes.

## Verification

Automated: settings round-trip for every new theme value; an old settings file with terminal theme keys loads with its app themes and saves without those keys; every `ThemeVariant` maps to a terminal palette whose `is_light` matches the variant; the Appearance page renders both galleries with six cards and no terminal-palette selections remain in the Settings page tests.

Live checks on macOS (run by `qa` in the development app):

- **UT-01 Galleries.** Settings → Appearance shows Theme and Window material, then the Light theme and Dark theme galleries with six cards each, matching `TV1e1`; the selected card has the ring and check, and the gallery for the current mode says “in use now”.
- **UT-02 Apply in dark mode.** With Theme on Dark and a shell chat open, picking each dark theme recolours the sidebar, panels and the open terminal together, without restarting the chat.
- **UT-03 Apply in light mode.** With Theme on Light, picking Tokyo Night Day and then Gruvbox Light recolours app and terminal together; switching Theme between Light and Dark swaps the whole theme and moves “in use now”.
- **UT-04 Terminal page.** Settings → Terminal has no Colors group; Text, Cursor, Keyboard and Scrollback & selection are unchanged.
- **UT-05 Migration.** A development settings file with `darkAppTheme: catppuccin-mocha`, `darkTerminalTheme: runner` and `lightTerminalTheme: catppuccin-mocha` opens with Catppuccin Mocha selected and its own terminal palette, and the file saved afterwards has no terminal theme keys. Back up the development settings first and restore them afterwards.
- **UT-06 Glass.** With Window material on Glass, Rosé Pine Moon and Solarized Light keep the sidebar text and selection readable.
