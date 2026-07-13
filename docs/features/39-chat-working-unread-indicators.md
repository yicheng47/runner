# 39 — Chat working and unread-completion indicators

> Tracking issue: [#285](https://github.com/yicheng47/runner/issues/285).

## Motivation

The sidebar folder/tab redesign in commit `46de90a` made every chat tab, whether single-pane or multi-pane, render through `ChatTabGroup`. That correctly removed individual pane rows from the sidebar, but it also removed the only visible activity signal: the old `SessionRow` leading dot mapped each session to `busy`, `idle`, `stopped`, or `crashed`. `Sidebar` still subscribes to `session/status`, but the activity map is now setter-only and no rendered component reads it.

Restoring that dot unchanged would restore the wrong product model. The sidebar should answer two workflow questions: **is this chat still working?** and **did it finish while I was elsewhere?** Raw lifecycle and idle status are already available inside the chat surface. The sidebar indicator should instead behave like an attention marker: a spinner at the trailing edge while work is active, followed by a dot only when the tab settles without being viewed.

This is tab-level state, not pane-level state. The latest sidebar hierarchy is Folder → Tab, and every pane in a tab is visible together on the chat surface. A multi-pane tab therefore has one aggregate working state and one aggregate viewed watermark.

## Investigation

- `src-tauri/src/session/pty_runtime.rs` infers `busy` and `idle` from PTY output. Any byte wakes a session to `busy`; 750ms of silence moves it to `idle`.
- Direct chats receive those transitions through the live-only `session/status` Tauri event. The event is not persisted and `SessionManager` exposes no current activity snapshot.
- A fresh sidebar subscriber therefore cannot reconstruct current state. Reusing the old `activity ?? "busy"` fallback would incorrectly show every already-running chat as working until its next transition.
- `src/components/Sidebar.tsx` still listens for `session/status`, but commit `46de90a` changed `directSessionActivity` to a discarded state value and stopped passing activity into `ChatTabGroup`.
- Durable chat tabs now live in SQLite (`tabs`, migration `0009`), while active-tab and pane focus remain per-window frontend state. The existing `WindowRegistry` already knows which direct-chat subjects each window displays and which window was most recently focused, but it does not retain an explicit currently-focused boolean after the app loses focus.
- There is no existing unread/read model for direct chat tabs. Mission inbox watermarks and the archived new-messages pill solve different problems and should not be reused.

## Scope

### In scope (v1)

- **Trailing tab indicator:** each `ChatTabGroup` row gains one fixed-size status slot immediately before the pane-count badge and kebab action. Render a small animated loading glyph while the tab is working, a solid accent dot when it has an unviewed completion, and nothing when it is settled and viewed. The trailing slot avoids bringing back the old leading lifecycle dot.
- **Tab-level working aggregation:** a tab is working when any running member session has live activity `busy`. The tab settles when its final busy member transitions to `idle`. Starting or resuming a session seeds that member as busy, so the spinner appears before the first output byte.
- **Unread completion:** a tab becomes unread only when it transitions from working to settled while no focused Runner window is displaying that tab. Initial hydration into an already-idle state does not synthesize a completion, and stop/crash/archive events do not create one.
- **Viewed semantics:** activating any pane in a tab marks the whole tab viewed because all of its panes share one visible surface. If a completion lands while that tab is visible in the currently focused Runner window, it is recorded as already viewed. If Runner is backgrounded, the completion is unread; returning focus to the still-visible tab clears it. Viewing the tab in any window clears the state globally.
- **Durable read watermarks:** add nullable `last_completed_at` and `last_viewed_at` columns to `tabs`. A tab is unread when completion is newer than view. New and imported tabs start with both values null. Persisting this state keeps all windows consistent and preserves an unviewed completion across navigation and app restart.
- **Live activity hydration:** retain the latest direct-chat activity in `SessionManager` and expose a batch `session_activity_snapshot` command. The sidebar subscribes before requesting the snapshot, buffers events received during the request, and applies those events after the snapshot so a transition cannot be overwritten by stale hydration.
- **Cross-window focus truth:** extend `WindowRegistry` entries with current focus state, updating it on both `Focused(true)` and `Focused(false)` plus hide/destroy. Completion handling can then distinguish a tab actually being viewed from one merely mounted in a background or hidden window.
- **Hidden-state rollup:** when a folder is collapsed, its row shows a spinner if any hidden child tab is working, otherwise a dot if any child is unread. The collapsed CHAT section header follows the same priority so status never disappears solely because the user collapsed its container. Expanded containers leave indicators on their tab rows and do not duplicate them on the parent.
- **Multi-window synchronization:** completion/view writes emit a dedicated tab-attention invalidation so every sidebar rehydrates the same timestamps without treating attention changes as layout mutations.

### State priority

For a tab, collapsed folder, or collapsed CHAT section, visible state uses this priority:

1. **Working:** animated loading glyph when any contained tab is working.
2. **Unread:** solid dot when nothing contained is working and at least one contained tab has an unviewed completion.
3. **Clear:** no indicator when everything is settled and viewed.

An unread completion is not cleared when new work begins. The spinner temporarily takes visual precedence; if the user still has not viewed the tab when work settles again, the dot returns with an updated completion watermark.

### Meaning of “done”

V1 uses the existing `busy` → `idle` PTY-silence transition as the completion signal. This means “the agent has settled or is waiting for input,” not a guaranteed semantic declaration that its logical task succeeded. A confirmation prompt and a finished response are both useful attention points. Runtime-specific prompt parsing or agent-reported completion would be more brittle and is not required for this workflow improvement.

### Out of scope

- Mission-row status changes. Missions keep their existing activity/lifecycle dots.
- Per-pane unread markers in the sidebar. Panes remain absent from the Folder → Tab hierarchy.
- Changes to the chat topbar, pane-header status pills, Stop/Resume controls, or stopped/crashed overlays.
- Counts such as “2 completed,” timestamps in the UI, sounds, OS notifications, or dock badges.
- Runtime-specific semantic completion parsing, OSC prompt detection, or a new agent protocol.
- Marking a user-initiated stop, crash, or archive as a successful completion.

## Key decisions

1. **Attention replaces lifecycle in the sidebar.** The old dot continuously described `busy`/`idle`/`stopped`/`crashed`; the new indicator is absent in the normal viewed-idle state and appears only for active work or something the user has not seen.
2. **The durable unit is the tab.** Sidebar rows, folders, names, ordering, and layouts are tab-based after feature 38. Read state belongs beside that data rather than on individual sessions or in per-window localStorage.
3. **Working overrides unread without clearing it.** One compact slot can communicate the immediately actionable state. Persisted watermarks retain older unviewed completion state under the spinner.
4. **Focused and visible is the viewed threshold.** Merely leaving a tab mounted in a background window must not swallow its completion dot. Conversely, a split tab that is on screen in the focused window is viewed even when another pane inside it owns keyboard focus.
5. **Hydration must be authoritative.** The current live-only event stream is insufficient for a persistent sidebar component. A backend snapshot removes the false-busy default and gives every window the same initial state.
6. **Idle remains the completion heuristic.** It already works across Claude Code, Codex, and future TUIs without agent cooperation. This feature changes how the signal is presented and remembered, not how terminal activity is inferred.

## Implementation Phases

### Phase 1 — design gate

- Mock single-tab, multi-pane-tab, unread, working, collapsed-folder rollup, and collapsed-CHAT rollup states in `design/chat-attention-indicators.pen` frame `R4LJz` before backend or frontend implementation.
- Lock the fixed trailing-slot placement, working-over-unread priority, and expanded-container non-duplication before coding.

### Phase 2 — queryable direct-chat activity

- Add latest activity to the per-session `SessionManager` state, seeded to busy for direct spawn/resume, updated before `session/status` emission, and cleared when the live handle exits. Spawn/resume also emits the seeded busy state immediately so a sidebar that completed hydration earlier does not wait for the PTY detector to wake from idle.
- Add `session_activity_snapshot` returning current live direct-session states by session id; stopped/crashed sessions are absent.
- Add a sidebar activity hook that subscribes first, hydrates second, and merges in-flight events after the snapshot. Unknown state renders no indicator while hydration is pending rather than defaulting to busy.
- Reuse pure tab aggregation helpers for the sidebar and tests; remove the setter-only `directSessionActivity` path and the duplicate sidebar-only display-status helpers once no longer used.

### Phase 3 — persisted attention and view state

- Add migration `0010_tab_attention.sql` with nullable `last_completed_at` and `last_viewed_at` columns, then extend `TabRow` and the frontend tab type.
- Add repository/command operations to record a settled transition and mark a tab viewed. Writes are monotonic, structural tab upserts preserve both watermarks, and attention writes emit `chat/tab-attention-changed` for cross-window rehydration.
- Extend `WindowRegistry` with explicit focused state and a helper that answers whether any currently focused window displays a member of a tab.
- On a direct-session idle transition, resolve its owning tab and aggregate current member activity. If this was the final busy member, record completion and mark it viewed in the same operation when the tab is focused and visible. Keep lifecycle exit paths out of completion recording.
- On tab activation, update the invoking window's visible subjects to every target-tab member and mark the tab viewed in one backend command before normal debounced route reporting can lag behind. Route navigation into an already-active tab and Runner-window focus returning to a visible tab also mark that tab viewed.

### Phase 4 — sidebar rendering

- Add the fixed trailing status slot to `ChatTabGroup`: compact accent spinner for working, 6px accent dot for unread, empty for clear. Keep the kebab as the final trailing control and avoid row-width movement between states.
- Aggregate child states for collapsed folder rows and the collapsed CHAT header with working-over-unread priority.
- Give indicators accessible labels/tooltips such as “Agent working” and “Completed — not viewed”; animation respects the existing reduced-motion behavior.

### Phase 5 — verification and documentation

- Add manager tests for busy seeding, activity snapshot updates, cleanup on exit, and a direct spawn or resume that occurs after the sidebar's initial snapshot.
- Add repository tests for completion/view watermark ordering, structural-upsert preservation, and migration defaults.
- Add pure frontend tests for single- and multi-pane aggregation, initial idle hydration, spinner precedence, unread retention under new work, and parent rollups.
- Add multi-window tests for focused-visible completion, background-window completion, switch-then-settle ordering, focus-return clearing, and cross-window invalidation.
- Update architecture documentation for direct-chat activity snapshots and tab attention persistence.

## Verification

- [ ] Start a new direct chat: its tab shows a trailing spinner immediately, before the first PTY output.
- [ ] With the sidebar already hydrated, start or resume another direct chat: the live busy seed shows its spinner without another snapshot.
- [ ] Let the active visible chat settle: the spinner disappears and no unread dot appears.
- [ ] Switch to another tab while the first works; when the first settles, its spinner becomes an unread dot.
- [ ] Activate any pane of the unread tab: the dot clears for that tab in every window.
- [ ] In a two- or three-pane tab, one busy member keeps the tab spinner visible; only the final busy → idle transition settles the tab.
- [ ] An unread tab that starts working again shows the spinner without losing its unread watermark; after settling, the dot returns until viewed.
- [ ] Stop, crash, or archive a working chat: no new completion dot is synthesized by the lifecycle change.
- [ ] Launch a window after sessions are already idle: snapshot hydration shows no false working spinners and does not create unread dots.
- [ ] Background Runner while a chat works, then let it settle: the tab is unread when Runner returns; focusing its already-visible tab clears the dot.
- [ ] View a tab in one Runner window: its unread dot clears in all other windows.
- [ ] Collapse a folder containing a working tab: the folder shows a spinner; after an unviewed completion it shows a dot. Expanding moves the signal back to the child row without duplication.
- [ ] Collapse the CHAT section: working/unread state remains visible on the section header with the same priority.
- [ ] Restart Runner with an unread completion: the dot restores from the tab watermarks; no migrated or newly-created tab starts unread.
- [ ] Indicator transitions do not shift the title, pane-count badge, or kebab control, and reduced-motion mode does not spin continuously.
- [ ] `pnpm exec tsc --noEmit`, `pnpm run lint`, targeted frontend tests, and `cargo test --workspace` pass.
