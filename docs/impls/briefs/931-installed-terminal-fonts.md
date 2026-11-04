# #931 — Installed terminal fonts

Jason approved this mission on 2026-11-03 for the `codex trio` crew. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-931-installed-terminal-fonts` on `feat/931-installed-terminal-fonts`, based on `origin/main` at `90940325`. The root checkout stays on `main`, and this worktree uses its own `target/`.

## Problem and outcome

The terminal offers only two fonts: the bundled JetBrains Mono Nerd Font Mono and the platform's system monospace font. A saved SF Mono, Fira Code or Monaco is silently collapsed into Menlo by serde aliases on `TerminalFontFamily`. People should be able to pick any monospace font installed on their computer. The outcome is a searchable Font family list in Settings → Terminal with **Bundled** (JetBrains Mono) and **Installed** (every monospace family found), a family-name setting that survives fonts being removed and reinstalled, and the same glyph fallbacks as today.

The spec is [`docs/features/931-installed-terminal-fonts.md`](../../features/931-installed-terminal-fonts.md); it and the [issue's 2026-11-03 comment](https://github.com/yicheng47/runner/issues/931#issuecomment-6598591359) hold the decisions. The list's look is the Font list on board `c518A` ("Files page · every option") of `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design`, without its "Same as terminal" row, which belongs to #634. The spec wins on behaviour, the design on appearance.

## Read first

- `TerminalFontFamily` in `crates/runner-app/src/app_settings.rs`, with its serde aliases, `family()` and `font()`.
- The terminal font settings in `crates/runner-app/src/app_store.rs` and how terminals pick up a font change.
- `render_terminal_settings` in `crates/runner-app/src/surfaces/settings/page/appearance.rs`, the font options in `surfaces/settings/page/mod.rs`, and the select and menu components in `crates/runner-app/src/ui/`.
- `crates/runner-app/src/surfaces/settings/theme_preview.rs`, the bundled-font registration in `crates/runner-app/src/assets.rs`, and `SYSTEM_MONOSPACE_FONT` and `TERMINAL_FONT_FALLBACKS` in `crates/runner-app/src/platform_ui/fonts_{macos,windows,linux}.rs`.
- GPUI's `TextSystem::all_font_names()`, and Zed's `FontFamilyCache` (`~/repos/gui/zed/crates/theme/src/font_family_cache.rs`) as the caching pattern.

## Deliverables

1. **Setting and migration.** Store the terminal font as a family-name string. A saved `JetBrains Mono` or `Meslo Nerd Font` loads as the bundled face; a saved `Menlo` loads as the platform's `SYSTEM_MONOSPACE_FONT`; `System default`, `Monaco`, `SF Mono` and `Fira Code` load as the family they name.
2. **Missing fonts.** A saved family that is not installed renders with the bundled face, keeps its saved name across saves, and shows as "Not installed" in the list until another font is chosen.
3. **Discovery.** Build the installed list once in the background from `all_font_names()`, cache it for the session, keep only families whose `i`, `W` and `m` advances match, sort by name, and show the bundled JetBrains Mono face only under Bundled.
4. **Settings.** Settings → Terminal → Font family becomes a searchable list: a search field, Bundled, then Installed, with the current choice checked. Reuse the existing select and menu components where they fit.
5. **Everywhere it shows.** Open terminals re-render with the new family and recompute their cell size; the Appearance theme preview uses it. The Nerd Font Mono face and `TERMINAL_FONT_FALLBACKS` stay as glyph fallbacks.
6. **Guide.** Update [Appearance](../../guide/guides/appearance.md), which sends readers to Settings → Terminal for the font, and its zh-CN counterpart if one exists.

Out of scope: the #634 file viewer and its Font setting, terminal ligatures, a changeable UI font (Inter stays), more bundled fonts, and any daemon, CLI or protocol change.

## Validation and handoffs

Name the changed behaviours and the check that covers each before running anything. In-process: unit tests for each legacy value's migration, the monospace filter accepting the bundled face and rejecting Inter, and a missing family resolving to the bundled face while its saved name survives a save; then the `runner-app` tests, workspace Clippy with warnings denied and `cargo fmt --all --check`, with the CI profile and `--locked`.

The coder implements; the reviewer reviews the diff and names the required live checks; qa runs them on the same candidate after the clean review, in the development app on macOS:
- the list shows Bundled then Installed, and search narrows it;
- choosing an installed font such as SF Mono or Menlo re-renders an open test terminal with correct cell widths, and Nerd Font icons and CJK text still render;
- a saved family that is not installed falls back to JetBrains Mono and keeps its name in Settings;
- the Appearance preview matches the terminal.

Windows live checks are left to Jason: label the PR `needs-live-test` and list them in its description. The final handoff carries the PR URL, final SHA, a short summary of changes and each check's result.

## Authorization

The `codex trio` crew may commit on this branch, push it and open one pull request against `main`, which stops there with CI green. Fold this brief and the spec into the implementation commits.
