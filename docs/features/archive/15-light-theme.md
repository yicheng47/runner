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
desktop.

A light variant is a small, scoped feature that closes that gap. The
design work covers two light options so the user can match how the
rest of their desktop is themed:

- **Codex Light** — pure-white surfaces, near-black ink, sky-blue
  accent. The Codex / ChatGPT desktop aesthetic. Frame `q92Ji`
  (Runners page) + `zS3Pe` (Mission workspace) +
  DS `YFawc` in `design/runners-design.pen`.
- **Solarized Paper** — Solarized Light surfaces with olive-green
  accent. Easier on the eyes in long sessions. Frame `pLbNm`
  (Mission workspace) + DS `iBOyT`.

Both ship in v1. Solarized was previously the only proposed light
variant; Codex was added after we saw users (incl. Jason's ByteDance
colleagues) treat Runner as a Codex-adjacent tool and expect the
chrome to match.

## Scope

### In scope (v1)

- **Theme tokens.** Add light-theme overrides of every variable in
  `src/index.css`'s `@theme` block. Two named light palettes:
  - `codex` (default light):
    - `--color-bg: #FFFFFF`
    - `--color-panel: #F7F7F8`
    - `--color-raised: #FFFFFF`
    - `--color-line: #E5E5E7`
    - `--color-line-strong: #D1D1D6`
    - `--color-fg: #1A1C1F`
    - `--color-fg-2: #6E6E73`
    - `--color-fg-3: #A0A0A8`
    - `--color-accent: #339CFF`
    - `--color-accent-ink: #FFFFFF`
    - `--color-accent-soft: #E6F2FF`
    - `--color-warn: #B45309`
    - `--color-warn-soft: #FEF3C7`
    - `--color-danger: #E5484D`
    - `--color-info: #0EA5E9`
  - `solarized`:
    - `--color-bg: #fdf6e3`
    - `--color-panel: #eee8d5`
    - `--color-raised: #fdf6e3`
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
- **Setting + selector.** Two storage keys in `src/lib/settings.ts`:
  - `theme: "auto" | "light" | "dark"` — top-level intent.
  - `lightVariant: "codex" | "solarized"` — which light theme to use
    when light is active. Default `"codex"`.
  - (`darkVariant: "carbon"` exists as a future hook but only has the
    one option for v1, so the UI just shows it locked.)
- **Settings UI.** New "Appearance" pane in `SettingsModal.tsx`
  (mock: frame `cxNBX` in `design/runners-design.pen`):
  - Theme segmented control: `Auto · Light · Dark`.
  - Light theme dropdown: `Codex Light` · `Solarized Paper`. Each
    row shows a 12×12 accent swatch + name. Live-applies on select.
  - Dark theme dropdown: `Carbon & Plasma` (disabled — only option).
  - "Tint brand mark" toggle: on by default. When on, the in-sidebar
    chevron uses `var(--color-accent)`; off pins it to the dark
    badge's `#00FF9C` regardless of theme.
- **Resolution.** `auto` reads `prefers-color-scheme` and listens
  for changes via
  `matchMedia("(prefers-color-scheme: light)").addEventListener(
  "change", …)`. App init writes the resolved pair to
  `<html data-theme>` (`codex` / `solarized` / `dark`).
- **In-sidebar brand mark recoloring.** The sidebar's `brandIcon`
  frame (currently three `#00FF9C` chevrons) follows the active
  theme's `var(--color-accent)`. Codex Light → `#339CFF`, Solarized
  → `#859900`, dark → `#00FF9C`. Pure CSS.
- **App icon stays constant.** The `.icns` ships unchanged; the dark
  badge with `#00FF9C` chevrons is the brand artifact on Dock /
  Cmd+Tab / notification center. Same decision as the previous draft
  of this spec.
- **Terminal theme stays separately configurable.** The existing
  `TerminalTheme` setting doesn't change. xterm.js takes a theme
  object programmatically; yoking it to the app theme creates UX
  surprises (a user who explicitly picked Dracula expects it to
  survive a chrome theme flip). A future spec can add a "follow app
  theme" option.

### Out of scope (deferred)

- **Per-mission / per-workspace theme.** v1 ships global only.
- **Custom-theme creator.** Pick-your-own-colors. No extension
  surface in v1.
- **High-contrast variants.** Real accessibility win, separate spec.
- **xterm "follow app theme."** Nice-to-have once both surfaces
  ship.
- **Theme-aware images / SVGs.** Handle per asset when needed; none
  today.

### Key decisions

1. **Tokens, not branches.** Each theme is a CSS variable override —
   same `bg-bg`, `bg-panel`, `text-fg`, `text-accent` utilities
   everywhere. No component renders different markup based on theme.
2. **`data-theme` on `<html>`, not React context.** Theme selection
   is a presentation concern; it shouldn't flow through component
   props. Writing the attribute at the root lets every component
   "react" via CSS without re-rendering.
3. **Pair-based selector, not flat list.** The user picks
   light-pair-when-light and (eventually) dark-pair-when-dark
   separately, instead of a flat list of {Carbon, Codex Light,
   Solarized}. This matches macOS / Linen / VS Code conventions
   where "Auto" needs to know what to switch *to* on each side.
4. **`auto` is the default for new installs.** Matches the OS
   preference on first launch. The persisted setting is the user's
   *intent* (`auto`/`light`/`dark` + `lightVariant`), not the
   resolved value.
5. **Codex is the default light variant.** Cleaner first impression
   and matches the Codex-adjacent workflow most users come from.
   Solarized is a one-click switch for users who prefer it.
6. **App icon does not theme.** The brand badge stays `#15161B` +
   `#00FF9C` chevrons everywhere it appears at a fixed asset level
   (`.icns`, Dock, system notifications, multi-window title chrome).
7. **Terminal theme stays decoupled.** xterm theme is configured
   independently; chrome theme switch doesn't touch it.

## Implementation phases

### Phase 1 — token overrides

- Extend `src/index.css`:
  - `[data-theme="codex"] { … }` with the Codex Light variables.
  - `[data-theme="solarized"] { … }` with the Solarized variables.
  - Existing dark block stays as the default (no `[data-theme]`
    selector needed — fallback when no attribute set).
- Verify scrollbar styling against the lighter panel colors; may
  need a per-theme override on `*::-webkit-scrollbar-thumb`.

### Phase 2 — setting + persistence

- Add to `src/lib/settings.ts`:
  - `STORAGE_APP_THEME = "settings.appTheme"` → `"auto" | "light"
    | "dark"`, default `"auto"`.
  - `STORAGE_APP_LIGHT_VARIANT = "settings.appLightVariant"` →
    `"codex" | "solarized"`, default `"codex"`.
  - `STORAGE_APP_BRAND_TINT = "settings.appBrandTint"` → bool,
    default `true`.
- Add `applyAppTheme()` helper:
  - Resolve effective surface: `auto` → check `prefers-color-scheme`,
    else use stored value.
  - If light: write `data-theme="<lightVariant>"`.
  - If dark: remove `data-theme` (dark is the unattributed default)
    *or* write `data-theme="dark"` if we add a dark variant later.
- Call `applyAppTheme()` once at app boot in `src/main.tsx` before
  React mounts.
- Subscribe `matchMedia("(prefers-color-scheme: light)")` to
  `change`; when intent is `auto`, re-apply on OS change.
- Same-window `storage` event sync via `notifySameWindowStorage` so
  the SettingsModal control flips the theme live without a reload.

### Phase 3 — SettingsModal Appearance pane

- New nav row in `SettingsModal.tsx`'s sidebar: "Appearance",
  Lucide `sun` icon, slotted between General and Terminal.
- Content layout matches the existing General pane (label + control
  rows, divider, preview).
- Controls:
  - Theme: segmented `Auto · Light · Dark` (Lucide
    `monitor`/`sun`/`moon` icons).
  - Light theme: `StyledSelect` with two options, accent-color
    swatch + name. Disabled when Theme = Dark.
  - Dark theme: `StyledSelect` with one option (Carbon & Plasma).
    Disabled in v1.
  - Tint brand mark: toggle. Switching it off pins the sidebar
    chevron to dark-green `#00FF9C`.

### Phase 4 — verification

- Visual smoke (manual):
  1. Fresh install, OS in light → Runner opens in Codex Light.
     Switch OS to dark → Runner flips to Carbon & Plasma within
     ~100ms.
  2. Appearance → Light theme = Solarized → Runner restyles
     without reload. OS theme toggle still works (auto follows OS,
     light variant honors the dropdown).
  3. Appearance → Theme = Light → Runner pins to the selected
     light variant regardless of OS.
  4. Tint brand mark off → sidebar chevron stays green even in
     Codex Light.
  5. Every page renders correctly in both light variants: Runners
     list, Crews list, Mission workspace, Direct chat, Settings
     modal, Search palette, Update toast, all other modals.
  6. AskHumanCard reads correctly in Codex Light (warn cream bg,
     amber border, blue primary button).
- Cross-spec compatibility:
  - Spec 14 (notifications): the `.icns` is unaffected.
  - Spec 12 (multi-window): new windows inherit `data-theme` from
    `<html>`.
- Tests: no new backend tests. Frontend unit test for
  `applyAppTheme()` covering the auto-resolution path + variant
  switch.

## Verification

- [ ] `data-theme="codex"` cascades the Codex Light variables.
- [ ] `data-theme="solarized"` cascades the Solarized variables.
- [ ] Default light variant is Codex.
- [ ] Light variant dropdown switches the theme live.
- [ ] `theme: "auto"` follows OS `prefers-color-scheme` at boot and
      on live OS change.
- [ ] Brand-mark tint toggle controls the sidebar chevron color.
- [ ] App icon `.icns` is unchanged.
- [ ] Terminal palette stays whatever the user picked in Terminal
      settings; theme switch doesn't override it.
- [ ] Every existing page + modal renders correctly in Codex Light
      and Solarized.
- [ ] AskHumanCard, status pills, accent buttons read correctly in
      all three palettes.
- [ ] `pnpm exec tsc --noEmit` clean; no backend changes.
