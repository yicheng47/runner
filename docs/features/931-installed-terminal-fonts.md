# 931 — Installed terminal fonts

> Tracking issue: [#931](https://github.com/yicheng47/runner/issues/931). Platforms: macOS and Windows; Linux keeps building and uses its platform text system for the list.
> Status: in development since 2026-11-03. Decisions are in the issue's [2026-11-03 comment](https://github.com/yicheng47/runner/issues/931#issuecomment-6598591359).
> Design: the Font list on board `c518A` ("Files page · every option") of `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design`. The terminal's list is the same without its first row, "Same as terminal".
> Related: [#634](https://github.com/yicheng47/runner/issues/634), whose file viewer reuses this list for its own Font setting.

## Motivation

The terminal offers two fonts: the bundled JetBrains Mono Nerd Font Mono and the system monospace font (Menlo on macOS, Consolas on Windows). People expect to use their own font, such as SF Mono, Fira Code, Iosevka, Berkeley Mono or Sarasa Mono for CJK. Earlier choices were collapsed into these two: the serde aliases on `TerminalFontFamily` map a saved SF Mono, Fira Code or Monaco to Menlo.

## Scope

- **The list.** Settings → Terminal → Font family becomes a searchable list: a search field, then **Bundled** with JetBrains Mono, the only font Runner ships, then **Installed** with every monospace family on this computer, sorted by name. The current choice is checked. No other fonts are bundled.
- **Finding installed fonts.** `cx.text_system().all_font_names()` lists the families on every platform (CoreText, DirectWrite, cosmic-text). The call is slow, so the list is built once in the background, the first time it is needed, and cached for the session, as Zed's `FontFamilyCache` does. The bundled JetBrains Mono face, which `add_fonts` registers, appears only under Bundled.
- **Monospace only.** A family counts as monospace when `i`, `W` and `m` have the same advance through the GPUI text system. This is one code path on every platform, instead of CoreText's monospace trait, DirectWrite's `IsMonospacedFont` and fontconfig's `FC_SPACING`. Inter and other proportional families never appear.
- **Storage and migration.** The setting becomes the family name as a string. A saved `JetBrains Mono` or `Meslo Nerd Font` loads as the bundled face. A saved `Menlo`, which until now meant the platform's system monospace font, loads as that platform's family (`SYSTEM_MONOSPACE_FONT`). The retired aliases `System default`, `Monaco`, `SF Mono` and `Fira Code` load as the family they name.
- **A missing font.** A saved family that is not installed at launch renders with the bundled face, and the saved name is kept, so the font returns when it is reinstalled; the next save does not overwrite it. The list shows the saved name with "Not installed" until another font is chosen.
- **Glyph fallbacks stay.** The grid renderer assumes every glyph fits one cell, so the bundled Nerd Font Mono and the platform `TERMINAL_FONT_FALLBACKS` remain fallbacks: icons and CJK still render with any primary font. CoreText looks fallbacks up by family name, so on macOS the bundled faces are also registered with CoreText at launch; otherwise another font's icons fall through to `.LastResort` boxes.
- **Everywhere the terminal font shows.** Running terminals and the Appearance theme preview use the chosen family, and changing it recomputes the cell size for open sessions.
- The UI font stays Inter and is not a setting. Ligatures stay out of the terminal, which shapes one cell at a time.

## Implementation phases

1. The setting as a family-name string, the migration from the old values, and font resolution with the missing-font fallback.
2. Discovery: the background, cached list from `all_font_names()` and the monospace filter.
3. The searchable Font family list in Settings → Terminal, and the theme preview.
4. The guide's [Appearance](../guide/guides/appearance.md) page, which points to Settings → Terminal for the font.

## Verification

- Unit tests: each legacy saved value loads as described; the monospace filter accepts the bundled JetBrains Mono face and rejects Inter; a missing family resolves to the bundled face while the saved name survives a save.
- Live, on macOS and Windows: the list shows Bundled then Installed, and search narrows it; choosing an installed font such as SF Mono or Cascadia Code re-renders open terminals with correct cell widths, and Nerd Font icons and CJK text still render; a saved font that is then removed falls back to JetBrains Mono and keeps its name in Settings; the Appearance preview matches the terminal.
