# Monokai terminal theme

Tracking issue: [#406](https://github.com/yicheng47/runner/issues/406). Status: planned. Respecced 2026-08-24 for the native GPUI line — the original spec targeted the Tauri app's `src/lib/settings.ts`, which is gone since the `v0.6.0` cutover.

## Motivation

The terminal theme picker ships Runner, Catppuccin Mocha, and Solarized Dark; Monokai — the palette the primary user actually wants in the panes daily — is missing. The theme system is built so variants are a pure additive change: a `TerminalPalette` const, an enum variant, and a dropdown option, with live repaint and canvas-background tracking driven from the existing machinery.

## Scope

- **`crates/runner-terminal/src/palette.rs`**: add `pub const MONOKAI: TerminalPalette` — classic Monokai, ANSI-16 mapped:

  | field | value |
  |---|---|
  | background | `#272822` |
  | foreground | `#F8F8F2` |
  | cursor | `#F8F8F2` |
  | cursor_accent | `#272822` |
  | selection | `#49483E` |
  | ansi 0–7 | `#272822` `#F92672` `#A6E22E` `#E6DB74` `#66D9EF` `#AE81FF` `#A1EFE4` `#F8F8F2` |
  | ansi 8–15 | `#75715E` `#F92672` `#A6E22E` `#F4BF75` `#66D9EF` `#AE81FF` `#A1EFE4` `#F9F8F5` |

- **`crates/runner-app/src/app_settings.rs`**: add `Monokai` to the `TerminalTheme` enum (`#[serde(rename_all = "kebab-case")]` persists it as `"monokai"`) and the `palette()` arm → `palette::MONOKAI`.
- **`crates/runner-app/src/surfaces/settings_page.rs`**: dropdown option `SelectOption::new("monokai", "Monokai").swatch(0xf92672)` — Monokai's signature pink, following the "most identifiable hue" swatch convention — plus the `terminal_theme_value` and `parse_terminal_theme` arms.
- No other UI changes: the Settings select renders from the option list; theme switches already repaint attached terminals live.

## Non-Goals

- Monokai app-chrome (light/dark variant) theming — terminal palette only.
- Monokai Pro / filter variants; one canonical Monokai.

## Implementation Phases

1. Palette const + enum variant + picker option.
2. Verification pass (below). No new components, no migrations.

## Verification

- `make verify`.
- Manual (Jason): Settings → Appearance → Terminal theme shows Monokai with the pink swatch; selecting it repaints open panes live (claude-code and codex TUIs legible: status lines, diffs, spinners); the pane background tracks `#272822` seamlessly; the selection persists across relaunch (`ui-settings.json` carries `"terminalTheme": "monokai"`).
