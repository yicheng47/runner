# 11 — Runner avatar (procedural identicon)

> Tracking issue: [#119](https://github.com/yicheng47/runner/issues/119)

## Motivation

Runner cards (rail, sidebar, mission rosters) are identified by a monospace
`@handle` string and a status dot. When a mission has three or four slots,
or the sidebar lists half a dozen runner templates, the eye has nothing to
lock onto except the text — every card is the same gray rectangle.

A small procedural identicon per runner — a "bits graph," GitHub-style
symmetric pixel grid deterministically generated from the handle —
fixes that at near-zero cost: the avatar is implicit (no picker, no
upload), it's stable per handle, and it surfaces everywhere the runner
is rendered. Multi-slot missions become scannable; "the green one is
the lead, the magenta one is the follower" is faster than reading two
monospace handles.

This is cosmetic, not functional. No new IPC, no migration in v1, no
asset pipeline. It's a frontend-only render component that hashes the
handle and paints a small SVG.

## Scope

### In scope (v1)

- **`<RunnerAvatar>` component.** New
  `src/components/ui/RunnerAvatar.tsx`. Props:
  - `seed: string` — the string used for the hash. Caller passes the
    runner's `handle` today (see [[Key decisions]] for why this is a
    `seed` rather than `handle`).
  - `size: "sm" | "md" | "lg"` — 16px / 24px / 40px.
  - Optional `className` for layout overrides only.
- **Procedural identicon body.** Render a 5×5 grid with vertical mirror
  symmetry (3 source columns mirrored to 5), GitHub identicon style:
  - Hash the seed with a small stable hash (FNV-1a, 32-bit).
  - First 15 bits → 3×5 source grid (bit=1 → painted cell).
  - Next 8–24 bits → fg color (HSL pick from a curated palette of
    ~12 hues with fixed S/L so contrast stays readable on the app's
    dark background).
  - Background stays transparent / inherits the card background;
    painted cells use the picked color.
  - Output: an inline SVG (no canvas, no image asset). Rounded
    corners on the outer frame to match the rest of the UI.
- **Stable hash function.** Document and freeze the algorithm in
  `src/lib/runner-avatar.ts` (FNV-1a 32-bit over UTF-8 bytes). Changing
  the algorithm later will reshuffle every avatar in the app, so it gets
  the same "don't change casually" treatment as the palette.
- **Render sites.**
  - `RunnersRail.tsx` card header — left of the `@handle` text, between
    the status dot and the handle (size `md`).
  - `Sidebar.tsx` runner section list rows — left of the runner name
    (size `sm`).
  - Mission slot roster wherever `SlotWithRunner` is rendered (size
    `md`). Exact file surfaces during impl.
  - Direct-chat / workspace header where today we render the bare
    `@handle` (size `md`).
- **Backwards compat.** Pure frontend; no DB change, no migration. Open
  the v0.1.11 app, then this build — avatars just appear.

### Out of scope (deferred)

- **User-pickable avatars.** Emoji input, image upload, color override,
  "regenerate." Out for v1. The point of an identicon is that it's
  automatic; if a user dislikes theirs, they can rename the runner
  (which today is a free operation). If real demand surfaces, add a
  `avatar_seed` column (see [[Key decisions]] #2) and let the user
  type a different seed string.
- **Animated identicons.** No.
- **Sharing avatars over the wire.** The router and event log don't
  carry avatars; rendering is local and joins on `runner.handle` /
  `slot.slot_handle`. Keeps the agent protocol clean.
- **Different identicon shapes** (hexagons, blobs, jdenticon-style
  triangles, etc.). 5×5 mirrored square is the cheapest readable shape
  at 16–24px; revisit only if it doesn't scan well in practice.

### Key decisions

1. **Procedural, not stored.** v1 has zero DB columns. The avatar is a
   deterministic function of the handle. This means renaming the runner
   changes the avatar — a real downside if users build up muscle memory
   for "the green one." Accepted for v1 because (a) renames are rare,
   (b) the v2 escape hatch (an `avatar_seed` column) is cheap to add
   later, and (c) zero-storage is the right default for a cosmetic.
2. **Component prop is `seed`, not `handle`.** Even though callers pass
   `handle` today, naming the prop `seed` keeps the door open for a v2
   where we read from a future `runners.avatar_seed` column when set
   and fall back to `handle` when NULL. Callers won't need to change.
3. **5×5 mirrored, not 8×8 or asymmetric.** Symmetric grids read as
   "intentional avatar" at small sizes; asymmetric noise reads as
   compression artifact. GitHub uses 5×5; that's the precedent and it
   works at 16px. Don't reinvent.
4. **Curated palette, not free HSL.** Fully random hues hit ugly muddy
   yellows and low-contrast cyans on a dark theme. A curated 12-entry
   palette with fixed S/L (think: the same color tokens used elsewhere
   in the UI plus a handful of jewel tones) guarantees every avatar
   reads cleanly on `bg`. Palette lives next to the component, easy to
   tune.
5. **SVG, not canvas.** Crisp at any size, no DPR handling, no
   `useEffect`. The whole component is a pure render of 25 `<rect>`s.

## Implementation phases

### Phase 1 — palette + hash + component

- `src/lib/runner-avatar.ts`:
  - `export function fnv1a(input: string): number` — 32-bit FNV-1a over
    UTF-8 bytes. Frozen contract.
  - `export const AVATAR_PALETTE: string[]` — ~12 hex colors tuned for
    the dark UI.
  - `export function identiconCells(seed: string): {
      cells: boolean[]; // length 25, row-major
      color: string;
    }` — pure function, deterministic.
- `src/components/ui/RunnerAvatar.tsx`:
  - Calls `identiconCells(seed)`, renders inline SVG with a 5×5 grid
    of `<rect>`s (or one `<path>` for compactness).
  - Size variants set `width`/`height` and the inner cell stride.
  - Rounded outer frame via `rx`.
- One unit test (`src/lib/runner-avatar.test.ts` if a test setup exists;
  otherwise document the expected output for two known seeds inline):
  given the seed `"orca"`, the function returns the expected cells +
  color. This locks the algorithm against accidental changes.

### Phase 2 — render sites

- `RunnersRail.tsx` card header: insert `<RunnerAvatar seed={s.handle}
  size="md" />` between the status dot and the handle text. Adjust gap
  classes so the row stays compact at the existing density.
- `Sidebar.tsx` runner list rows: insert `<RunnerAvatar seed={handle}
  size="sm" />` at the start of each row, before the runner name.
- Mission slot roster (likely `MissionInput.tsx` and the workspace
  mission header — confirm during impl): `<RunnerAvatar
  seed={slot.runner.handle} size="md" />` next to each `slot_handle`.
- Direct-chat / workspace header for a runner session: same component
  before the `@handle` rendering.

### Phase 3 — verification

- Frontend smoke (dev server):
  1. Open the app with the existing seed set of runners. Every card in
     the rail, sidebar, mission roster, and chat header shows a unique
     identicon; identicons for the same handle match across all four
     surfaces.
  2. Create a new runner `@orca`. Its avatar appears immediately, no
     reload needed.
  3. Rename a runner from `@foo` to `@bar`. Avatar changes (documented
     v1 behavior — see [[Key decisions]] #1).
  4. Visual check at 16px (sidebar): the grid is still parseable, the
     color is distinguishable from neighboring rows.
- No backend tests required (no schema or command changes).

## Verification

- [ ] Identicons render at all four surfaces (rail card, sidebar row,
      mission slot roster, direct-chat header).
- [ ] Same handle produces the same identicon everywhere in the app.
- [ ] No `pnpm tauri build` or DB-migration step needed to adopt the
      feature on an existing install.
- [ ] At 16px the grid + color remain readable on the dark theme.
- [ ] `pnpm exec tsc --noEmit` clean; `cargo test --workspace`
      unaffected (no backend changes).
