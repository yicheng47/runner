# Full-page Settings redesign + keyboard shortcuts pane + in-app update prompt

## Status

Planned. Design complete in `design/runner-setting.pen` (all node IDs below reference that file). Delivers feature #257 (keyboard shortcut settings page) as its v1 read-only scope, and supersedes the modal-era settings design in `runner-mvp-design.pen` (the "Settings modal — *" frame family there is legacy once this ships).

## Problem

Settings today is a 680×560 modal (`src/components/SettingsModal.tsx`, ~1500 lines) with eight cramped panes. It has no room to grow: the keyboard shortcuts reference (#257) doesn't fit a modal, the Updates pane is a whole tab for what is essentially one button and one toggle, and update discovery is a transient `UpdateToast` that deep-links back into the modal. The redesign moves settings to a full-window page in the ChatGPT-desktop style — sidebar navigation with grouped sections, wide content column with card-grouped rows — merges Updates into About, adds the Keyboard shortcuts pane, and replaces the toast with an Arc-style update prompt card anchored above the sidebar's Settings row on the main page.

## Design (source of truth: `design/runner-setting.pen`)

Eight screens, left to right on the canvas: General (`S4fHyW`), Keyboard shortcuts (`mOfwl`), Chat (`n1Evo`), Appearance (`b3v7u`), Terminal (`wSSjz`), MCP (`Zhmly`), Diagnostics (`aIjFr`), About (`SA5su`). Plus two spec artifacts: the About update-button state ladder (`IKGNz`) and the in-app update prompt card in sidebar context (`e1jYEa`), each with an adjacent annotation note (`IHwvB`, `USjVJ`).

Layout: settings sidebar (280px in the design canvas; implemented resizable, sharing the app sidebar's width store so it opens at the main page's sidebar width — decided post-review) with Back to app button, "Search settings…" field, nav groups **App** = General / Chat / Appearance / Terminal / Keyboard shortcuts, **Integrations** = MCP, **System** = Diagnostics / About, + content column (~880px, left-padded 120). All colors map 1:1 to existing CSS tokens in `src/index.css`: sidebar `--color-sidebar` #272930, active nav `--color-sidebar-selected` #333640 + `-border` #3b3e49, cards `--color-panel` on `--color-bg`, chips/controls `--color-raised`, hairline row dividers #24262C. Design rule established with the user: **neutral for states, accent for meaning** — the only accent-green elements are toggles-on, the READY update CTA, and the update prompt card's button.

## Key Decisions

1. **Settings is a route, not a modal.** `/settings/:pane?` renders its own two-column surface without the app `Sidebar` (the settings sidebar replaces it in the same slot — resizable, sharing the app sidebar's persisted width — so the takeover reads as continuous; the sidebar fill matching `--color-sidebar` is deliberate). "Back to app" returns to the previous location (store the `location` the user came from; fall back to `/`). Per-window, like any route (impl 0018 unaffected).
2. **Pane components move, logic stays.** `SettingsModal.tsx` already isolates each pane as a function component (`GeneralPane`, `ChatPane`, `AppearancePane`, `TerminalPane`, `McpPane`, `DiagnosticsPane`, `UpdatesPane`, `AboutPane`) over shared `Row`/`StyledSelect`/`Toggle` primitives. Extract panes to `src/components/settings/`, re-skin the row shell to the card + hairline pattern (new `SettingsCard` / `SettingsRow`), and keep every setting's read/write logic untouched. The modal is deleted at the end, not forked.
3. **Updates merges into About; the Updates tab dies.** About leads with a hero card: app icon (`design/app-icon.png` = the real icon asset), name + version chip, status line, and the stateful update button beside the logo. Button ladder per spec `IKGNz`: IDLE "Check for updates" (neutral) → CHECKING (dimmed, disabled) → AVAILABLE "Download vX.Y.Z" (quiet accent: #0F2418 fill, accent text/border) → DOWNLOADING (dimmed + 3px accent progress bar under the hero) → READY "Restart to update" (solid accent — the one solid-accent moment in settings). "Install updates automatically" toggle sits under the hero; Links card (GitHub / Documentation / License) below. **About auto-checks on mount** — without a dedicated tab, a manual-only check means nobody ever sees AVAILABLE.
4. **The Arc-style prompt card replaces `UpdateToast`.** When updater status is ready-to-restart, the main page's app sidebar shows a floating card directly above the Settings row (spec `e1jYEa`): full-bleed header band "New Runner version available" (#363845 strip over a top-lit radial-gradient card #31333D→#1E1F25, divider below), an "Automatic updates" checkbox, and a full-width slim (26px) "Restart and Update" button with a center-glow accent gradient. Dismissable per launch (state in memory or `sessionStorage`); reappears next launch until installed. The card's checkbox and About's toggle are the same persisted setting — one storage key, two surfaces. `UpdateToast` and its `onOpenSettings` deep-link are deleted.
5. **Keyboard shortcuts pane ships read-only (v1) on a static keymap registry.** New `src/lib/keymap.ts`: one exported list of `{ id, title, description, keys, scope }` covering the real bindings — ⌘N new window, ⌘T new chat, ⌘K command palette, ⌘S toggle sidebar (⌘\ legacy alias noted in the entry, not rendered), ⇧⌘[/⇧⌘] page navigation, ⌘+/−/0 zoom, ⌘[/⌘] pane focus + ⌘W close pane (split only), ⌘1 feed / ⌘2–⌘9 slots (mission workspace). The pane renders title, a "Search shortcuts" filter field, and one flat card of rows with mono key chips. The registry is presentation-only in v1 — handlers keep their hardcoded keys; the registry documents them. Drift risk is accepted and noted in each handler via a one-line comment pointing at `keymap.ts`.
6. **Rebinding is designed, not built.** The design specs the end-state interactions (hover-reveal pencil/trash per row, chip-click → "Press keys…" recording state as a neutral inset well, unbind → "Unassigned", implied "Restore defaults") — see the states on the `mOfwl` rows and note `IHwvB`. v1 renders resting rows only. Customizable bindings are a follow-up feature with real scope (capture UI, conflict detection, persistence, handler indirection) — explicitly out.
7. **Settings search filters navigation, not row content.** The sidebar search field filters nav items by pane label match in v1. Row-level search (ChatGPT-style) needs a searchable settings index; deferred.
8. **⌘, opens settings.** Standard macOS binding, added to the keymap registry and wired app-side; the sidebar Settings row and command palette entry navigate to the route instead of setting `settingsOpen`.
9. **The General pane's "Remember window position" row ships only with #271.** It appears in the design as a preview; v1 of this impl renders General without it. Do not implement window-state persistence here.

## Goals

- Settings opens as a full-window page with the designed sidebar (grouped nav, search, back button) and card-styled panes; the modal is gone with zero settings-behavior regressions.
- Keyboard shortcuts pane lists the real keymap, searchable, read-only (#257 v1 delivered).
- About = hero with real app icon + version + five-state update button, auto-check on mount, auto-install toggle, links; no Updates tab anywhere.
- Update-ready surfaces as the sidebar prompt card on the main page; toast deleted; "Restart and Update" restarts into the new version.
- Light theme works by construction (all styling through existing semantic tokens).

## Non-Goals

- Shortcut rebinding/unbinding/recording (designed; separate follow-up feature).
- Row-level settings search.
- Window position persistence (#271).
- Any change to what settings exist or how they persist (`localStorage` keys, backend commands stay as-is).
- Porting the legacy modal frames in `runner-mvp-design.pen` (left as historical reference).

## Implementation Phases

### Phase 1 — extraction refactor (no visual change)

- Move the eight pane components + shared row primitives out of `SettingsModal.tsx` into `src/components/settings/` modules; `SettingsModal` becomes a thin shell importing them. Typecheck/lint gate; app behavior identical.

### Phase 2 — the page shell

- `src/pages/SettingsPage.tsx` + route `/settings/:pane?` in `App.tsx`, rendered outside `AppShell`'s sidebar layout (own two-column surface, drag region for the title bar preserved).
- Settings sidebar per design: back button (stored return location), search field (nav filter), grouped nav with active states.
- `SettingsCard` / `SettingsRow` shells per the card + hairline design; port panes onto them.
- Entry points: sidebar Settings row + command palette navigate to the route; ⌘, binding; delete `settingsOpen` plumbing (`AppShell.tsx`, `Sidebar.tsx`). Keep `SettingsModal.tsx` compiling but unreferenced; delete it at the end of the phase once parity is confirmed.

### Phase 3 — keymap registry + Keyboard shortcuts pane

- `src/lib/keymap.ts` registry (vitest: no duplicate ids/keys; every entry has scope).
- `ShortcutsPane`: search field filters rows by title/description/key; flat card of rows with mono chips; pointer comments added at each hardcoded handler.

### Phase 4 — About/Updates merge

- Hero card with `app-icon.png`, version chip (existing version fetch), status line + `UpdatesAction` ladder restyled per spec `IKGNz`; auto-check on pane mount via `useUpdate()`; progress bar in DOWNLOADING; auto-install toggle row; Links card. Remove the Updates pane and its nav entry.

### Phase 5 — update prompt card

- `UpdatePromptCard` in the app `Sidebar` above the Settings row, gated on `useUpdate()` ready state + per-launch dismiss; header band / gradient / glow button per spec `e1jYEa`; checkbox bound to the same auto-install setting; delete `UpdateToast`.

### Phase 6 — cleanup + docs

- Delete `SettingsModal.tsx`; sweep dead references; update `docs/arch/arch.md` (settings surface + update flow sections).
- Checks: `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace` (unchanged backend — smoke only), vitest for keymap.
- Manual smoke (user-run): open settings from sidebar / palette / ⌘,, walk all eight panes, flip a setting in each and confirm persistence, resize window, back-button returns to prior page, second window opens its own settings; About auto-checks and walks the ladder against a staged update; prompt card appears on ready, dismiss and relaunch behavior, restart installs.

## Relevant Code

- `src/components/SettingsModal.tsx` — all pane logic to extract; `PANES` list (~line 132); `Row`, `UpdatesAction`, version fetch.
- `src/contexts/UpdateContext.tsx` + `src/hooks/useUpdateChecker.ts` — shared updater state (`useUpdate()`); already single-source, feeds About + prompt card.
- `src/components/UpdateToast.tsx`, `src/components/AppShell.tsx` (toast mount, `settingsOpen`, ⌘S handler), `src/components/Sidebar.tsx` (Settings row, ⌘K/⌘T handlers, prompt card mount point).
- `src/App.tsx` — routes, zoom handlers; `src/pages/MissionWorkspace.tsx`, `src/pages/RunnerChat.tsx` — remaining shortcut handlers for the keymap registry.
- `src/index.css` — token source; no new tokens needed.
- `design/runner-setting.pen` — spec; `design/app-icon.png` — hero asset.

## Open Questions

- **Search scope growth**: if nav-label filtering feels too thin in practice, the row-level index is the natural v2 — decide after dogfooding.
- **Prompt card for AVAILABLE (not yet downloaded) when auto-install is off**: current spec shows the card only at READY. If auto-install is off, nothing proactive surfaces AVAILABLE outside About. Acceptable for v1 (auto-install defaults on); revisit if it bites.

## References

- Feature #257 — keyboard shortcut settings page (delivered as v1 read-only by this impl).
- Feature #271 — window position persistence (previewed in the General design; not in scope).
- Design: `design/runner-setting.pen` (all screens + spec strips + annotation notes).
- Visual reference: ChatGPT desktop settings (layout language), Arc browser update card (in-app prompt pattern).
