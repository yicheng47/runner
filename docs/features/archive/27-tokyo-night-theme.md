# 27 — Tokyo Night dark theme

> Tracking issue: [#204](https://github.com/yicheng47/runner/issues/204)

## Motivation

Runner already has a proper light/dark theme model: the user picks the top-level intent (`Auto`, `Light`, or `Dark`) and separately picks which palette to use for the resolved light or dark surface. The dark side currently has Runner and Catppuccin Mocha. Tokyo Night should join that dark-theme list because it is a familiar developer palette with calmer blue-purple surfaces, strong contrast, and less neon than Runner's default Carbon chrome.

Catppuccin Mocha should stay a dark theme. Catppuccin Latte is the light counterpart. Moving Mocha into the light side would make the theme model semantically wrong and would break the existing pair: Latte for light, Mocha for dark.

## Scope

### In scope

- Add `Tokyo Night` as a third dark app theme option in Appearance.
- Keep the top-level theme model unchanged: `Auto`, `Light`, and `Dark` still pick the resolved surface, and `Dark theme` chooses the dark palette for dark mode.
- Add a `[data-theme="tokyo-night"]` CSS variable override in `src/index.css`.
- Add `tokyo-night` to `DarkVariant`, `DARK_VARIANT_OPTIONS`, labels, and accent swatches in `src/lib/settings.ts`.
- Use the Tokyo Night palette as chrome tokens:
  - `--color-bg: #1a1b26`
  - `--color-panel: #24283b`
  - `--color-raised: #292e42`
  - `--color-line: #414868`
  - `--color-line-strong: #565f89`
  - `--color-sidebar: #16161e`
  - `--color-sidebar-selected: #24283b`
  - `--color-sidebar-selected-border: #414868`
  - `--color-fg: #c0caf5`
  - `--color-fg-2: #a9b1d6`
  - `--color-fg-3: #565f89`
  - `--color-accent: #7aa2f7`
  - `--color-accent-ink: #16161e`
  - `--color-warn: #e0af68`
  - `--color-danger: #f7768e`
  - `--color-info: #7dcfff`
- The Appearance preview should work through the existing `data-theme` preview path.

### Out of scope

- No new terminal palette in this feature. Terminal themes are separately configured in the Terminal pane.
- No "follow app theme" terminal behavior.
- No custom theme editor.
- No changes to the light theme list.
- No migration of existing users' dark theme choice; Runner remains the default dark variant.

### Key decisions

1. **Tokyo Night is a dark app chrome variant.** It belongs beside Runner and Catppuccin Mocha in `Dark theme`, not beside Codex Light and Catppuccin Latte in `Light theme`.
2. **Use CSS tokens only.** Components keep using `bg-bg`, `bg-panel`, `text-fg`, `text-accent`, and the existing theme cascade.
3. **Default stays Runner.** This is an opt-in palette, not a brand reset.
4. **Terminal remains independent.** A user can run Tokyo Night chrome with Runner, Catppuccin Mocha, or Solarized Dark terminal colors.

## Implementation Phases

### Phase 1 — dark variant settings

- Extend `DarkVariant` in `src/lib/settings.ts` to include `"tokyo-night"`.
- Add `"tokyo-night"` to `DARK_VARIANT_OPTIONS`.
- Add `DARK_VARIANT_LABELS["tokyo-night"] = "Tokyo Night"`.
- Add `DARK_VARIANT_ACCENTS["tokyo-night"] = "#7AA2F7"`.
- Keep `DEFAULT_DARK_VARIANT = "carbon"`.

### Phase 2 — CSS token override

- Add `[data-theme="tokyo-night"]` to `src/index.css` with the scoped token set above.
- Keep Carbon as the unattributed default. Tokyo Night should only apply when `applyAppTheme()` writes `data-theme="tokyo-night"`.
- Verify sidebar selected rows, active tabs, borders, status pills, warning surfaces, and accent buttons against the darker surface ramp.

### Phase 3 — settings UI validation

- Confirm the existing Appearance `Dark theme` dropdown shows `Runner`, `Catppuccin Mocha`, and `Tokyo Night`.
- Confirm the swatch is Tokyo Night blue.
- Confirm the preview card updates immediately when Tokyo Night is selected.
- Confirm `Theme = Auto` uses Tokyo Night when the OS is dark and `Dark theme = Tokyo Night`.
- Confirm `Theme = Dark` pins Tokyo Night regardless of OS.

## Verification

- [ ] `data-theme="tokyo-night"` cascades the Tokyo Night variables.
- [ ] Appearance → Dark theme includes `Tokyo Night`.
- [ ] Switching to Tokyo Night updates the app without reload.
- [ ] `Theme = Auto` uses Tokyo Night when the OS resolves dark.
- [ ] `Theme = Light` still ignores the dark variant and uses the selected light theme.
- [ ] Catppuccin Latte remains a light theme; Catppuccin Mocha remains a dark theme.
- [ ] Terminal theme is unchanged when the app chrome switches to Tokyo Night.
- [ ] Runners, Crews, Mission workspace, Direct chat, Settings, Command palette, and modals are readable in Tokyo Night.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
