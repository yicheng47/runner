# 02 — Collapsable sidebar

> Tracking issue: [#34](https://github.com/yicheng47/runner/issues/34)

## Motivation

The sidebar is permanently fixed-width (resizable, but never hidden).
On smaller laptop screens — or any time the user wants the mission
workspace / chat to fill the screen for a focused task — that ~256px
column is just dead space. There is no way to hide it.

Every comparable tool (VS Code, Cursor, Slack, Linear) lets the user
toggle a sidebar between full and a narrow icon-rail. We should match
that affordance.

## Scope

### In scope (v1)

- Two-state collapse: **full** (current behavior, user-resizable) ↔
  **icon rail** (~52px wide, icon-only).
- Toggle methods:
  - Chevron button at the bottom of the sidebar in both states —
    on the right of the Settings row when expanded, as its own row
    above Settings when in rail mode.
  - Keyboard shortcut `cmd+\\` (mac) / `ctrl+\\` (win/linux).
- State persists in `localStorage` via a new
  `STORAGE_SIDEBAR_COLLAPSED` key in `src/lib/settings.ts`, using the
  same `"1"` / `"0"` encoding the existing settings use.
- Icon-rail layout (top → bottom):
  - Brand mark (no "Runner" wordmark)
  - WORKSPACE icons: runner, crew, search — each with a tooltip
    showing its label on hover; click navigates the same as the full
    row.
  - Archived icon (once feature #31 lands; coordinate so both can
    ship without conflict).
  - Settings icon at the bottom.
  - MISSION and SESSION lists are **entirely hidden** when
    collapsed. They are label-heavy (titles, pin/active state,
    rename affordances, kebab menus) and do not reduce well to
    icons. Picking a mission or chat requires expanding.
- The custom title-bar drag region (`data-tauri-drag-region` h-7
  strip at the top) stays in place at both widths.
- Collapse animates over ~150ms (width transition). Long enough to
  feel intentional, short enough not to feel laggy.
- Resize-drag handle is disabled while collapsed (no width to drag),
  re-enables on expand. The remembered full-width is preserved
  across collapse/expand cycles.

### Out of scope (deferred)

- **Pinned missions/sessions surfaced in the rail.** Tempting (a few
  most-recent or pinned items as icon avatars) but doubles the rail
  layout work and is fragile when a user has 0–N pins. v1 hides the
  list entirely; we can iterate later.
- **A third "fully hidden" state.** Users on truly small windows can
  drag the bottom-right corner to widen the workspace; the rail is
  thin enough that we don't need a zero-width mode.
- **Auto-collapse on window resize.** Out of scope; behavior is
  user-driven only.
- **Mobile / touch-friendly drawer behavior.** Runner is a desktop
  app; not needed.

### Key decisions

1. **Icon rail, not full hide.** Anchors muscle memory, lets users
   expand without hunting for an off-screen handle, matches VS Code /
   Cursor / Linear / Slack conventions.
2. **`cmd+\\` over `cmd+B`.** `cmd+B` is reserved by some browsers
   and overloaded with "bold" in any future text-input surface;
   `cmd+\\` is unambiguous and matches one of the conventions used
   by other dev tools.
3. **MISSION/SESSION hidden, not iconized.** Listing 5 missions as
   identical chevron icons is worse than hiding them — there's no
   cheap way to convey title/pin/active state in 32px.
4. **One persisted bit, not separate width-when-collapsed.** Width
   in collapsed state is constant (52px); only the binary
   collapsed/expanded flag is persisted. The user's preferred full
   width stays remembered separately by the existing resize logic.

## Implementation phases

### Phase 1 — Settings storage + AppShell wiring

- Add `STORAGE_SIDEBAR_COLLAPSED` constant to `src/lib/settings.ts`.
- Hoist `collapsed` state into `AppShell.tsx` (alongside `settingsOpen`)
  so a future top-level shortcut handler can toggle it.
- Pass `collapsed` + `onCollapsedChange` down into `Sidebar`.
- Bind `cmd+\\` / `ctrl+\\` at the AppShell level via a
  `useEffect` keyboard listener — mirrors the pattern used in any
  other top-level shortcut.

### Phase 2 — Sidebar dual layout

- Inside `Sidebar.tsx`, branch on `collapsed`:
  - When `false`: current layout, no functional change.
  - When `true`: render `<aside style={{ width: 52 }}>` with the
    icon-only rail (brand → WORKSPACE icons → Archived → spacer →
    Settings).
- Each icon wrapped in a tooltip on hover — reuse whatever tooltip
  primitive is in use today (or keep it lightweight with a `title`
  attribute if no primitive exists yet).
- Disable the resize handle (`<div onMouseDown=…>`) when collapsed.
- Width transition: tailwind `transition-[width] duration-150`.

### Phase 3 — Toggle button + keyboard shortcut polish

- Chevron toggle lives at the bottom of the sidebar in both states.
  - **Open state**: 24×24 button on the right of the bottom Settings
    row (justify-between with the Settings link). Icon:
    `ChevronsLeft` (lucide). Click → collapse.
  - **Rail state**: 36×36 button as its own row above the Settings
    icon, separated from the WORKSPACE icon stack by a 1px divider.
    Icon: `ChevronsRight` (lucide). Click → expand.
- Focus order in both states: `brand → first nav row → … →
  settings → toggle` — the toggle is the last interactive element
  in the aside.
- Keep the title-bar drag region (`data-tauri-drag-region` h-7
  strip) at the top of the aside in both states. The toggle no
  longer interacts with this strip — they don't share a row
  anymore — so no special pointer-events handling is required.

### Phase 4 — Pencil design + visual polish

- Design both states (full + rail) in `design/runners-design.pen`
  before locking in spacing and icon sizes. Confirm tooltip style
  matches the existing kebab-menu / row-hover design language.

## Verification

- [ ] `cmd+\\` toggles the sidebar between full and rail in either
      direction.
- [ ] Clicking the chevron does the same.
- [ ] State persists across app restart (close, relaunch, sidebar
      stays in last state).
- [ ] In rail mode, every WORKSPACE / Archived / Settings icon
      tooltips its label on hover and navigates correctly on click.
- [ ] Resize handle works when expanded, is inert when collapsed,
      and the user's last full-width is restored after expand.
- [ ] No regression in the existing per-section MISSION / SESSION
      collapse (those still toggle within the expanded sidebar).
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
