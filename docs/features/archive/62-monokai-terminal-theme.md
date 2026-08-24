# Monokai terminal theme

Tracking issue: [#406](https://github.com/yicheng47/runner/issues/406). Status: shipped in [v0.6.3](https://github.com/yicheng47/runner/releases/tag/v0.6.3) — PR #446 → `974a158`, 2026-08-24, by mission on `codex-crew`; the smoke test grew the scope to the Monokai Pro values, the Runner canvas lift, the Solarized Dark retirement, and the two select/popup fixes below. Respecced 2026-08-24 for the native GPUI line — the original spec targeted the Tauri app's `src/lib/settings.ts`, which is gone since the `v0.6.0` cutover.

## Motivation

The terminal theme picker shipped Runner, Catppuccin Mocha, and Solarized Dark; Monokai replaces Solarized Dark as the third option. Live repaint and canvas-background tracking already come from the existing theme machinery.

## Scope

- **`crates/runner-terminal/src/palette.rs`**: add `pub const MONOKAI: TerminalPalette`, ANSI-16 mapped:

  | field | value |
  |---|---|
  | background | `#2D2A2E` |
  | foreground | `#FCFCFA` |
  | cursor | `#C1C0C0` |
  | cursor_accent | `#8E8D8D` |
  | selection | `#5B595C` |
  | ansi 0–7 | `#2D2A2E` `#FF6188` `#A9DC76` `#FFD866` `#FC9867` `#AB9DF2` `#78DCE8` `#FCFCFA` |
  | ansi 8–15 | `#727072` `#FF6188` `#A9DC76` `#FFD866` `#FC9867` `#AB9DF2` `#78DCE8` `#FCFCFA` |

- **`crates/runner-terminal/src/palette.rs`**: raise the existing Runner palette background from `#15161B` to the app panel color `#1D1E23` and its selection highlight from `#272930` to `#3B3E49`; keep ANSI black and `cursor_accent` at `#15161B`.
- Unknown persisted terminal-theme labels load as Runner without resetting unrelated settings.

- **`crates/runner-app/src/app_settings.rs`**: add `Monokai` to the `TerminalTheme` enum (`#[serde(rename_all = "kebab-case")]` persists it as `"monokai"`) and the `palette()` arm → `palette::MONOKAI`.
- **`crates/runner-app/src/surfaces/settings_page.rs`**: dropdown option `SelectOption::new("monokai", "Monokai").swatch(0xff6188)` — the palette's signature pink, following the "most identifiable hue" swatch convention — plus the `terminal_theme_value` and `parse_terminal_theme` arms.
- **`crates/runner-app/src/ui/select.rs`**: the native smoke test exposed top-aligned content in single-line select options; center those rows while preserving top alignment for detailed or described two-line options.
- **`crates/runner-app/src/ui/menu.rs`**: popup panels occlude covered hitboxes so choosing an Agent in Start Chat cannot open the Model suggestions underneath.
- No further UI changes: the Settings select renders from the option list; theme switches already repaint attached terminals live.

## Non-Goals

- Monokai app-chrome (light/dark variant) theming — terminal palette only.
- Additional Monokai variants; one canonical palette only.

## Implementation Phases

1. Palette const + enum variant + picker option, including the Runner background lift and unknown-label fallback.
2. Smoke-test corrections for single-line select-option alignment and popup hitbox occlusion.
3. Remove the retired Solarized Dark option from the shipped palette list.
4. Verification pass (below). No new components, no migrations.

## Verification

- `make verify`.
- Manual (Jason): Settings → Appearance → Terminal theme shows Monokai with the pink swatch; selecting it repaints open panes live (claude-code and codex TUIs legible: status lines, diffs, spinners); the pane background tracks `#2D2A2E` seamlessly; the selection persists across relaunch (`ui-settings.json` carries `"terminalTheme": "monokai"`).
- Manual (Jason): the picker contains Runner, Catppuccin Mocha, and Monokai only; an unknown persisted terminal-theme label falls back to Runner while preserving other preferences.
- Manual (Jason): switching back to Runner repaints the terminal canvas at `#1D1E23`, matching the app panel instead of the deeper `#15161B` app background.
- Manual (Jason): selecting text in a Runner pane shows the lighter `#3B3E49` highlight clearly against the canvas. The same palette `selection` color drives the mission feed's markdown highlight (`mission_workspace.rs:3179`), so check a feed with highlighted text under both Runner and Monokai as well.
- Manual select regression (Jason): the terminal-theme swatch, label, and checkmark align vertically; add-slot runtime, new/edit runner permission, edit-runner effort, and start-chat effort keep their described two-line rows top-aligned.
- Manual popup regression (Jason): in Start Chat → Direct, changing Agent updates the available models without opening the Model dropdown.
