# 15 — Light theme

> Tracking issue: [#133](https://github.com/yicheng47/runner/issues/133)

## Motivation

Runner ships with one theme — Carbon & Plasma (carbon surfaces, neon
`#00FF9C` accent). It's defined in `src/index.css` as a single
`@theme` block of CSS custom properties. That's fine for dev tooling
that gets used in a dark room, but real users work outdoors, in
co-working spaces, on plane seats, with their OS in light mode for
everything else. The current single-theme story forces a context
switch every time Runner is one of many open windows on a light
desktop — visually jarring and inconsistent.

A light variant is a small, scoped feature that closes that gap. The
design work is already done: `design/runners-design.pen` frame
`iBOyT` (DS · Option 3 — Solarized Paper) and the paired Mission
workspace mock at frame `pLbNm` define the full palette and how it
applies to every surface.

Solarized Light was chosen because (a) it preserves the accent green's
identity across themes — `#859900` is the direct Solarized parity for
`#00FF9C`, so brand continuity holds, and (b) Solarized's palette is
already designed for light/dark parity, so we get a fully-spec'd
secondary color set (blue, yellow, red) without inventing one.

## Scope

### In scope (v1)

- **Theme tokens.** Add a light-theme override of every variable in
  `src/index.css`'s existing `@theme` block. Use the Solarized Light
  palette frozen in the DS:
  - `--color-bg: #fdf6e3`
  - `--color-panel: #eee8d5`
  - `--color-raised: #fdf6e3` (with border for elevation)
  - `--color-line: #d8d0b3`
  - `--color-line-strong: #93a1a1`
  - `--color-fg: #002b36`
  - `--color-fg-2: #586e75`
  - `--color-fg-3: #93a1a1`
  - `--color-accent: #859900`
  - `--color-accent-ink: #fdf6e3`
  - `--color-warn: #b58900`
  - `--color-danger: #dc322f`
  - `--color-info: #268bd2`
- **Setting + selector.** New `theme: "auto" | "light" | "dark"`
  setting in `src/lib/settings.ts`. App init writes the resolved value
  to `<html data-theme>`. `auto` reads `prefers-color-scheme` and
  listens for changes via `matchMedia("(prefers-color-scheme: light)").
  addEventListener("change", …)`.
- **Settings UI.** New "Appearance" row in the General pane of
  `SettingsModal.tsx` with a 3-way segmented control: `Auto · Light ·
  Dark`. Persists via the same `notifySameWindowStorage` pattern the
  terminal settings already use.
- **In-sidebar brand mark recoloring.** The sidebar's `brandIcon`
  frame (currently three `#00FF9C` chevrons) follows the theme: olive
  `#859900` in light, neon green in dark. Pure CSS — fill via
  `var(--color-accent)`.
- **App icon stays constant.** The `.icns` ships unchanged; the dark
  badge with `#00FF9C` chevrons is the brand artifact on Dock /
  Cmd+Tab / notification center. Decision documented at
  `design/runners-design.pen` frame `iBOyT` ("APP ICON" section).
- **Terminal theme stays separately configurable.** The existing
  `TerminalTheme` setting in `src/lib/settings.ts` doesn't change.
  Users who want a light PTY palette pick one explicitly in the
  Terminal settings pane. A future spec can add an "Auto" terminal
  theme that follows the app theme; out of scope here.

### Out of scope (deferred)

- **Per-mission / per-workspace theme.** Some users will want
  mission A in light, mission B in dark. v1 ships global only.
- **Custom-theme creator.** Pick-your-own-colors. v1 ships two
  shipped themes plus auto; no extension surface.
- **High-contrast variants.** Real accessibility win, real scope
  expansion. Separate spec when needed.
- **xterm "auto" theme.** Auto-pair the terminal theme to the app
  theme. Nice-to-have once both surfaces ship.
- **Theme-aware images / SVGs.** If we ever embed raster assets that
  need a light variant (illustrations, empty-state graphics), handle
  per asset. None today.

### Key decisions

1. **Tokens, not branches.** The light theme is a CSS variable
   override — same `bg-bg`, `bg-panel`, `text-fg`, `text-accent`
   utilities everywhere. No component renders different markup based
   on the theme. This is what the existing Tailwind v4 `@theme` block
   was set up for; we're using the affordance.
2. **`data-theme` on `<html>`, not a React context.** Theme selection
   is a presentation concern; it shouldn't flow through component
   props or context. Writing the attribute at the root lets every
   component "react" via CSS without re-rendering. The setting hook
   only needs to update the attribute, not the tree.
3. **`auto` is the default for new installs.** Matches the OS
   preference on first launch. Users who want to force a theme pick
   it explicitly in Settings. The persisted setting is the
   user's *intent* (`auto`/`light`/`dark`), not the *resolved value*
   — that way OS theme changes still flip Runner when intent is auto.
4. **App icon does not theme.** The brand badge stays
   `#15161B` + `#00FF9C` chevrons everywhere it appears at a fixed
   asset level (`.icns`, Dock, system notifications, multi-window
   title chrome). The in-app sidebar mark recolors because it's
   rendered live in HTML/CSS as part of a themed UI surface — that's
   a different artifact. See DS comment at `iBOyT`'s APP ICON section.
5. **Terminal theme stays decoupled.** xterm.js takes a `theme`
   object programmatically; it's not driven by CSS variables. Users
   pick a terminal palette independently (today's behavior). Yoking
   PTY palette to the chrome theme creates UX surprises (e.g., a
   user explicitly picked Dracula for the terminal, expects it to
   survive flipping the chrome to light).

## Implementation phases

### Phase 1 — token overrides

- Extend `src/index.css`: add `[data-theme="light"] { … }` rule with
  the Solarized Light variable overrides listed in [[Scope]].
- Update body `background-color` + `color` to use the same variables
  (already do). Verify scrollbar styling reads OK against
  `#fdf6e3` — may need to tune `*::-webkit-scrollbar-thumb` to a
  lighter `#d8d0b3` via the same variable approach.

### Phase 2 — setting + persistence

- Add `theme: "auto" | "light" | "dark"` to `src/lib/settings.ts`
  alongside the terminal settings. Storage key
  `STORAGE_APP_THEME`, default `"auto"`.
- Add `applyAppTheme()` helper that:
  - Resolves the effective theme: `auto` → check
    `prefers-color-scheme`, else use stored value.
  - Writes `<html data-theme="…">`.
- Call `applyAppTheme()` once at app boot (in `src/main.tsx` before
  React mounts).
- Subscribe `matchMedia("(prefers-color-scheme: light)")` to
  `change`; when intent is `auto`, re-apply on OS change.
- Same-window `storage` event sync (existing
  `notifySameWindowStorage` pattern) so the SettingsModal control
  flips the theme live without a reload.

### Phase 3 — SettingsModal UI

- New row in the General pane: label "Appearance", value = segmented
  control `Auto · Light · Dark`.
- Visual style: same affordance as the existing zoom / terminal
  font-size rows.
- On change: write to settings, dispatch the synthetic storage event,
  call `applyAppTheme()`.

### Phase 4 — verification

- Visual smoke (manual):
  1. Fresh install, OS in light → Runner opens in light. Switch OS
     to dark → Runner flips to dark within ~100ms.
  2. SettingsModal → set to "Light" → Runner stays light regardless
     of OS.
  3. Set to "Dark" → Runner stays dark.
  4. Set back to "Auto" → follows OS again.
  5. Every page renders correctly in both themes: Runners list,
     Crews list, Mission workspace, Direct chat, Settings modal,
     Search palette, Update toast, all modals.
  6. Status pills, accent buttons, AskHumanCard, RunnersRail
     busy/idle dots all render in their themed accent.
- Cross-spec compatibility:
  - Spec 14 (notifications): the icon attached to a notification is
    the `.icns`, unaffected. Notification content text inherits
    macOS's own theming.
  - Spec 12 (multi-window): new windows inherit `data-theme` from
    `<html>`; each window's React tree picks up the cascade.
- Tests: no new backend tests required (this is pure frontend +
  localStorage). Frontend unit test for `applyAppTheme()` covering
  the resolve-from-`auto` path.

## Verification

- [ ] `data-theme="light"` cascades the Solarized variables across
      every component.
- [ ] `theme: "auto"` follows OS `prefers-color-scheme` at boot and
      on live change.
- [ ] Switching theme in SettingsModal flips the app live (no
      reload).
- [ ] App icon `.icns` is unchanged; sidebar brand mark recolors via
      `var(--color-accent)`.
- [ ] Terminal palette stays whatever the user picked in Terminal
      settings; theme switch doesn't override it.
- [ ] Every existing page + modal renders correctly in light:
      Mission workspace, direct chat, Runners / Crews, Settings,
      Search palette, Update toast.
- [ ] AskHumanCard, status pills, accent buttons read correctly with
      both palettes.
- [ ] `pnpm exec tsc --noEmit` clean; no backend changes.
