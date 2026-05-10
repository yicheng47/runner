# 07 — App zoom & terminal font size in General settings

> Tracking issue: [#78](https://github.com/yicheng47/runner/issues/78)

## Motivation

Two recurring asks from users on different display sizes:

1. **The whole UI feels too small (or too big) on this monitor.** A 4K
   external next to a 13" laptop, or a Studio Display vs. a 14" MacBook
   — the same `text-[12px]` density that reads cleanly on one screen
   is unreadable on another. Users want a single "make everything
   bigger" knob like browsers and IDEs ship.
2. **The runner terminal text specifically is too small / too big.**
   `RunnerTerminal.tsx:156` hardcodes `fontSize: 13` into the xterm
   config. Even after a global UI zoom, the terminal canvas is xterm's
   own renderer and needs its own font-size knob to stay crisp; a
   webview zoom on a canvas-rendered terminal blurs.

The General pane in Settings is the right home: it already owns
"Defaults and startup behavior" and is where users go to look first.
Two new rows: one for **App zoom** (whole-UI scale) and one for
**Terminal font size** (xterm-only).

## Scope

### In scope (v1)

- **App zoom row** in `GeneralPane` (`SettingsModal.tsx`):
  - Control: a `−` / `100%` / `+` cluster matching the pencil-design
    visual vocabulary (or a `StyledSelect` with discrete steps —
    decided in Phase 1). Steps: 80%, 90%, 100%, 110%, 125%, 150%.
    Default 100%.
  - Mechanism: `getCurrentWebview().setZoom(level)` from
    `@tauri-apps/api/webview`. Persisted in localStorage at
    `settings.appZoom` (string, e.g. `"1.1"`).
  - Applied at app boot (a small effect in `App.tsx` reads the stored
    value and calls `setZoom` once on mount) and immediately on
    change.
- **Terminal font size row** in `GeneralPane`:
  - Control: a `StyledSelect` with `Small (12)`, `Default (13)`,
    `Large (15)`, `Extra large (17)`. Default `Default (13)`.
  - Persisted in localStorage at `settings.terminalFontSize`
    (number, integer).
  - Consumed by `RunnerTerminal.tsx`: replace the hardcoded
    `fontSize: 13` with a read of the stored value at terminal
    construction time.
- **Live update on terminals already open**: when the user changes
  the terminal font size with a runner chat already mounted, the
  open terminal should pick up the new size. Use a `storage` event
  listener (or a tiny shared signal) so existing
  `RunnerTerminal` instances call `term.options.fontSize = next`
  followed by `fitAddon.fit()`.
- **Settings storage helpers**: extend `src/lib/settings.ts` with
  `STORAGE_APP_ZOOM` / `STORAGE_TERMINAL_FONT_SIZE` constants and
  typed getters (`readAppZoom()`, `readTerminalFontSize()`). Keep
  `src/lib/settings.ts` the single source of truth, matching how
  `STORAGE_AUTO_INSTALL_UPDATES` works today.

### Out of scope (deferred)

- **Per-surface font size** for the EventFeed message body, the
  workspace headers, etc. The codebase uses pixel-locked classes
  (`text-[12px]`, `text-[13px]`, …) almost everywhere; making those
  scale via a CSS variable means refactoring every text class. App
  zoom covers the common case. If a user wants only the message body
  bigger and not the chrome, that's a Phase 2 with a real
  rem-or-CSS-var refactor.
- **Cmd-+ / Cmd-− keyboard shortcuts.** Discoverable, useful, and
  cheap, but separate from the settings surface; can land as a
  follow-up against the same storage key.
- **Per-window zoom state.** Runner is single-window; revisit only if
  multi-window lands.
- **Sync across machines.** All settings are local-only today; no
  cloud sync infrastructure exists yet.

### Key decisions

1. **Two distinct settings, not one.** Webview zoom handles DOM-rendered
   surfaces correctly but blurs xterm's canvas. xterm's own
   `fontSize` produces crisp glyphs but doesn't affect the rest of
   the UI. They're load-bearing in different layers, so they get
   different controls. Naming makes the distinction visible:
   "App zoom" vs. "Terminal font size".
2. **Discrete steps, not a free slider.** Six zoom steps and four
   font-size steps cover the realistic range and avoid users
   landing on weird non-integer pixel sizes that subpixel-rasterise
   poorly. Keeps the UI quiet, too.
3. **Tauri `setZoom` API, not CSS `zoom`.** CSS `zoom` works in
   WebKit/Chromium but is non-standard; the Tauri API is the
   sanctioned cross-platform path and survives webview swaps.
4. **localStorage, not the backend.** The General pane already
   persists every setting in localStorage today (default crew,
   default working dir, auto-install). Adding a backend settings
   table just for these two rows is more infra than the surface
   warrants. When the settings store lands as its own work, these
   keys migrate alongside the others.
5. **Apply zoom on boot from one place.** A single effect in
   `App.tsx` reads the stored zoom and calls `setZoom` once. Putting
   the apply logic next to the read in `SettingsModal` would only
   work while the modal is mounted — boot needs a separate
   apply-on-mount entry.

## Implementation phases

### Phase 1 — storage + boot apply

- Add `STORAGE_APP_ZOOM` / `STORAGE_TERMINAL_FONT_SIZE` constants and
  typed read/write helpers to `src/lib/settings.ts`.
- In `App.tsx` (or `main.tsx`, whichever owns first-render side
  effects today), add a one-shot effect that reads the stored zoom
  and calls `getCurrentWebview().setZoom(stored)`. No-op on `1.0`.

### Phase 2 — `GeneralPane` rows

- Two new `<Row>`s in `GeneralPane`:
  - `App zoom` — `−` / `<percent>` / `+` cluster (decision: stepper
    cluster matches the rest of the modal; alternative is
    `StyledSelect` with the discrete options). Picks one and
    documents the choice in the implementation PR.
  - `Terminal font size` — `StyledSelect` over the four named
    sizes.
- Both write through to localStorage immediately on change.
- App zoom also calls `setZoom(next)` so the change is felt
  immediately; the persisted value is the ground truth on next boot.

### Phase 3 — `RunnerTerminal` consumption + live update

- `RunnerTerminal.tsx`: on mount, read
  `readTerminalFontSize()` and feed it into the xterm constructor in
  place of the literal `13`.
- Subscribe to a `storage` event (or a small in-process
  `EventTarget`-based signal — pick one and document) so an open
  terminal can react to the user changing the size in the modal:
  `term.options.fontSize = next; fitAddon.fit()`.
- Verify dimensions don't drift: an in-flight stream + size change
  shouldn't lose lines or scramble the cursor position.

## Verification

- [ ] Open Settings → General → change App zoom from 100% to 110% →
      the whole UI scales immediately, including the modal itself.
      Reopen Settings: 110% is still selected.
- [ ] Quit and relaunch the app: the UI starts at 110% without the
      modal open.
- [ ] Reset to 100%: UI restores; localStorage key cleared (or set
      to `"1"`, decided in Phase 1).
- [ ] Open a runner chat with a streaming session, change Terminal
      font size to Large: text re-rasterises to 15px, no dropped
      output, cursor stays correct.
- [ ] Change font size with no terminal mounted: open a new runner
      chat afterwards → new chat picks up the stored size on first
      render.
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
