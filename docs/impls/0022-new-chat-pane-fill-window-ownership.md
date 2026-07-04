# New-Chat Pane Fill & Multi-Window Ownership (direct-chat split view)

## Status

Investigation + fix design for a multi-window dogfooding bug in the direct-chat split view (impl [0020](0020-direct-chat-split-view.md), built on multi-window impl [0018](0018-multi-window.md)). This doc is the impl log; the fix itself is a small, localized frontend change (see [Fix](#fix)). Branch: `fix/new-chat-window-ownership` (rebased on `origin/main` @ `39c7587`, which already carries the group-pin-inheritance work).

## Problem (user repro, deterministic — "happens each time")

With more than one Runner window open, in a split tab (chat group in this doc's original vocabulary — see the terminology follow-up at the bottom):

1. Archive one pane's chat — the pane empties in place, the other pane keeps its chat.
2. Hit **New chat**.
3. The newly-created session does **not** land in the current window's empty pane. The split silently collapses to a single full-screen chat, and the work the user was in "ends up belonging to a separate window."

Expected: the new session fills the emptied pane, the split stays intact, and the acting window keeps owning every chat it shows.

## Root cause

**The sidebar new-chat entry point navigates to the spawned session without making it a member of the active group.** It is the only new-session path that skips the group.

There are three ways a fresh or existing chat is placed into a split, and the design invariant (`RunnerChat.tsx:926-931`) names exactly three explicit membership-add acts — the preset picker, the empty-pane `StartChatModal`, and a sidebar pick into a focused empty pane. Two of the three assign into a pane; the sidebar **new-chat** path does not:

| Entry point | Handler | Assigns into a pane? |
|---|---|---|
| Empty-pane "New chat" button (path a) | `RunnerChat.tsx:1769` `StartChatModal.onStarted` | ✅ `assignSessionToPane(target, spawned.id)` then `focusPane` |
| Sidebar row **pick** into empty pane | `Sidebar.tsx:799-845` `openDirectChat` | ✅ `assignSessionToPane(focusedPaneId, …)` when the focused pane is empty |
| **Sidebar "new chat" row + ⌘T (path b)** | `Sidebar.tsx:1090-1098` `StartChatModal.onStarted` | ❌ **only `navigate(\`/chats/${spawned.id}\`)`** |

`⌘T` (`Sidebar.tsx:406-410`) and the "new chat" nav row (`Sidebar.tsx:924`, `:983`) both route to the same `creatingChat` modal, whose `onStarted` only navigates. So the most natural "hit New chat" gesture is the one broken path.

### Why that produces the collapse and the "separate window" symptom

The split is a *chat group* — a binding between specific sessions, not a viewport mode. `RunnerChat` renders the group only while the open chat is a member:

```
// RunnerChat.tsx:294
const splitActive = isGroupActiveFor(layout, sessionId);   // false for a non-member
```

Because path (b) never adds `spawned.id` to the layout, `isGroupActiveFor(layout, spawned.id)` is **false**, so the group backgrounds and the new chat renders single-pane. The emptied pane is abandoned — that is the visible "the new chat didn't land in the pane" half.

The multi-window half follows from how a window's *ownership* is derived. Each window reports the subjects it shows via `useReportSubjects` (`RunnerChat.tsx:293-320`), and that list is computed from `splitActive`:

```
// RunnerChat.tsx:313-317
const paneSessionIds = splitActive ? visibleSessionIds(layout.root) : [];
const subjectIds =
  sessionId && !paneSessionIds.includes(sessionId)
    ? [sessionId, ...paneSessionIds]
    : paneSessionIds;
```

Before the new chat, the acting window reported every pane's session (e.g. `[X, Y]`, then `[X]` after archiving `Y`). The instant path (b) backgrounds the group, `splitActive` flips to `false`, `paneSessionIds` becomes `[]`, and the window reports **only the new session** — it *drops* the subjects for the group members it was still showing. In the backend `WindowRegistry` the most-recently-focused holder of a subject is its owner (`windows.rs:127` `primary_for`; mirrored frontend by `isSecondaryFor`, `windowFocus.ts:143`). The natural reason to have two windows open is that one of the split's chats is also open in the other window; the acting window was that chat's owner (it was focused last). When path (b) makes the acting window stop reporting it, the *other* window becomes the sole/primary holder and its `DuplicateSubjectOverlay` clears — the chat the user was driving "jumps" to the separate window, while the new chat is stranded solo.

### What is *not* the root cause (verified, so the fix stays localized)

- **The arbitration is sound.** `WindowRegistry` (`windows.rs`) and `isSecondaryFor` (`windowFocus.ts:143-166`) resolve ownership by `focused_at` correctly; the existing unit tests cover the tie/promotion/hidden-main cases. No registry change is needed, and the fix must **not** touch the overlay.
- **No window ever *adopts the new session*.** A window's registry subjects are set exclusively by its own `window_report_subjects` call, driven only by its own URL + layout store. Nothing — not the backend spawn (`commands/session.rs:510` `session_start_direct` never touches `state.windows`), not any `session/*` event listener, not the shared-but-only-`main`-persisted layout store (`paneLayout.ts:400-410`) — causes a *second* window to navigate to or assign the new session. So the new session is always reported by the acting window alone, which is focused, so it is always the new session's owner; the overlay can never fire on the *new* session itself. The observed "belongs to a separate window" is the *group members'* ownership migrating away, per the mechanism above — a refinement of the initial hypothesis that framed the overlay as landing on the new session.
- **The debounce (`windowFocus.ts:77-91`) is not the trigger.** It coalesces last-write-wins per window; the acting window's final reported set is correct. The collapse is a pure membership-logic outcome, which is why it is deterministic ("happens each time") rather than timing-dependent.

### One detail the fix must respect: the emptied pane is *not* focused

`archiveSession` (`RunnerChat.tsx:1217-1271`) leaves focus on a live pane in both cases — archiving a background pane keeps focus on the URL-owner (no navigation), and archiving the URL-owner focuses the *survivor* and navigates to it (`:1244-1253`). Either way the URL sits on a live member and the emptied pane is left **unfocused**. So a fix that fills only the *focused* empty pane (as `openDirectChat` does at `:822`) would not fire for this repro — the fix must target the first empty pane in the active group, matching what the empty-pane button already does (it targets a specific, possibly-unfocused pane, `ChatPaneGroup.tsx:250` → `RunnerChat.tsx:1730`).

## Fix

Make the sidebar new-chat `onStarted` fill the active group's empty pane, mirroring path (a) and the pick path. Concretely, in `Sidebar.tsx` `StartChatModal.onStarted` (`:1093`):

1. Read the store layout (`getPaneLayout()`).
2. If the current chat's group is active (`isGroupActiveFor(layout, currentChatSessionId)`) and has an empty pane, `assignSessionToPane(firstEmptyPaneId, spawned.id)` + `focusPane(firstEmptyPaneId)`, and inherit the group pin the same way `openDirectChat` does (`shouldInheritPinOnAdd` / `api.session.pin`), so the new member stays in the group's sidebar cluster.
3. Navigate as today.

When no group is active or there is no empty pane, behavior is unchanged (plain navigate to a single-pane chat).

Because the new chat becomes a member, `splitActive` stays `true`, the split keeps rendering, the emptied pane is filled, and the acting window keeps reporting `[…, spawned.id]` for every pane — so it retains ownership of all of them and no other window adopts or is handed anything. This addresses the root cause (group membership), not the symptom (it does not suppress the overlay).

### Extract a pure helper for testability

The pane-fill decision is pure and belongs in `paneLayout.ts` next to the existing helpers, so it can be unit-tested in the `paneLayout.test.ts` style without React:

```ts
/** The pane a new chat should fill: the first empty pane of the active
 *  group for `currentChatSessionId`, or null when there is no active
 *  group / no empty pane (caller falls back to plain navigation). */
export function newChatTargetPane(
  layout: PaneLayout,
  currentChatSessionId: string | null,
): string | null
```

Scope `newChatTargetPane` to the **sidebar new-chat path only** — it is the path with no specific pane in hand, so "first empty pane of the active group" is the right rule for it. Do **not** route the empty-pane button through it: that path already carries an explicit clicked-pane id (`ChatPaneGroup.tsx:249-250` → `RunnerChat.tsx:1769-1777`), and collapsing it to "first empty pane" would misfire in a 3-pane layout with two empty panes (clicking the *second* empty pane would fill the *first*). The empty-pane button and the pick path stay as they are; only the sidebar new-chat path gains the helper. Keep the change minimal and localized — no registry or pane-layout refactor beyond this helper — so it merges cleanly with any concurrent split-view work.

## Verification

- **New unit tests** (`paneLayout.test.ts`) for `newChatTargetPane`: returns the emptied pane after a background-pane archive; returns the emptied pane after a URL-owner archive (survivor focused, empty pane unfocused); returns `null` for a single-pane chat and for a full group with no empty pane; returns `null` when the open chat is a non-member (group backgrounded).
- **Existing gates green:** `pnpm exec vitest run`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `cargo test --workspace`. (No Rust change, so `windows.rs` coverage is unchanged and already sufficient — the arbitration is not the defect.)
- **Manual two-window smoke** (for the human — do not launch the GUI from the agent):
  1. Window M: open chat X, split 2-col, fill pane 2 with Y. Open chat X (or Y) in a second window S via the sidebar's "Open in new window" so S shares a member; click back into M so M owns its panes.
  2. In M, archive one pane (the pane empties in place; the other keeps its chat; focus stays on the live pane).
  3. In M, hit New chat via the sidebar row **and** via ⌘T.
  4. **Before:** the split collapses, the new chat opens full-screen in M, the emptied pane is gone, and the member shared with S becomes driven by S (S's overlay clears).
  5. **After:** the new chat fills the emptied pane, the split stays intact, M shows no `DuplicateSubjectOverlay`, M still owns every pane, and S's state is untouched.

## Relevant code

- `src/components/Sidebar.tsx:1090-1098` — sidebar new-chat `StartChatModal.onStarted` (**the bug**: navigate-only).
- `src/components/Sidebar.tsx:406-410`, `:924`, `:983` — ⌘T + "new chat" row → the same `creatingChat` modal.
- `src/components/Sidebar.tsx:799-845` — `openDirectChat` pick path (fills the *focused* empty pane — the precedent to mirror).
- `src/pages/RunnerChat.tsx:1765-1783` — empty-pane button `StartChatModal.onStarted` (path a; already fills its target pane).
- `src/pages/RunnerChat.tsx:1217-1271` — `archiveSession` (leaves the emptied pane unfocused, URL on a live member).
- `src/pages/RunnerChat.tsx:293-320` — `subjectIds` / `useReportSubjects`: why backgrounding the group drops the members' subjects.
- `src/pages/RunnerChat.tsx:294`, `src/lib/paneLayout.ts:92-101` — `splitActive` / `isGroupActiveFor` (non-member ⇒ group backgrounds).
- `src/lib/paneLayout.ts:180-238`, `:482-494` — `applyPresetPure`, `assignSessionPure`/`removeSessionPure`, `assignSessionToPane`/`focusPane` (where `newChatTargetPane` lands).
- `src/lib/windowFocus.ts:100-166` — `useReportSubjects`, `isSecondaryFor` (arbitration; sound — not the defect).
- `src-tauri/src/windows.rs`, `src-tauri/src/commands/window.rs`, `src-tauri/src/commands/session.rs:510` — registry + spawn (verified: spawn never assigns window ownership).

## Follow-up: terminology correction — the "separate window" was a TAB

Continued dogfooding resolved a vocabulary mismatch that shaped this whole doc: in the user's reports, "window" meant **a new group of panes on the same chat surface** — what the hierarchy now calls a **tab** (arch §3.6, Window → Tab → Pane) — not an OS window. There was never a second OS window in the repro. Consequences:

- The multi-window ownership mechanism analyzed above is real machinery and the analysis stands, but it was not what the user was reporting. The observed bug in the user's vocabulary — "the new chat lands in a separate window" — reads correctly as: **the new chat opens as a new single-pane tab instead of filling the empty pane of the current tab.**
- An interim ⌘N → "File → New Chat" menu rebinding (built on the OS-window misreading) was reverted; ⌘N remains **New Window**, matching the hierarchy (⌘N = window, ⌘T = tab, panes fill from the pane's New chat button or a sidebar pick).
- The sidebar new-chat fill shipped above is correct and stays. The still-open defect is that the **empty-pane New chat button** (`RunnerChat`'s pane modal, path a) reportedly also lands the chat in a new tab, despite `assignSessionToPane` + `focusPane` + navigate reading correct. That path is instrumented (`[pane-fill]` console logs around the assign) pending a captured repro; the fix lands here once the logs identify the failing step.
- A later regression made the implementation treat the window as having only one persistent pane tab, so creating panes from a non-member chat replaced the previous pane tab. That was corrected by storing pane tabs as a per-window set keyed by member sessions: non-member pane creation now adds another tab, and reopening any member re-activates its existing tab.

## Follow-up: one-by-one resume scrollback corruption

After an app restart that restored a multi-pane **tab** with every **pane** stopped, resuming panes one by one could duplicate and reflow-garble Codex banner frames in scrollback while live prompt regions stayed clean. The same panes resumed cleanly through topbar **Resume all**, which made the failure specific to the per-pane resume sequence in an already-visible tab, not to backend replay or the resume fork dimensions.

The captured writer is `src/components/RunnerTerminal.tsx:737-779`, `refitAndPush()` calling `pushSize()`, whose `src/components/RunnerTerminal.tsx:692-710` path writes the local `ESC[2J ESC[H` clear for Codex/Claude runtimes and immediately sends `session_resize`. The proof lines were the enabled-pane refits to transient collapsed grids, each followed by `push-size`: `56x40 -> 56x19 disabled=false` then `push-size ... rows=19`, `46x18 -> 46x7 disabled=false` then `push-size ... rows=7`, and another `46x18 -> 46x7 disabled=false` then `push-size ... rows=7`. That path was not the activation wake dance (`dance=false` and no `resize-dance` lines during the settle path); it was a later resize observer/window resize refit hitting siblings that were already enabled in the one-by-one flow.

Root cause: `refitAndPush()` mutated xterm with `fit.fit()` before deciding whether the proposed grid was stable. During the resume sequence, a whole-viewport/pane-height transient proposed a much shorter row count for every visible pane. Because the panes were enabled, `pushSize()` treated that transient as real: it cleared the visible xterm region, pushed the collapsed PTY size, and Codex repainted at the tiny height. When the layout returned to the real size, the tiny-height repaint and clear had already been committed into scrollback, producing the duplicated/reflow-garbled banner frames. Resume all avoided the bug because sibling panes stayed transitional/disabled during the burst, so this enabled-pane writer did not run for them.

Fix: gate `refitAndPush()` before `fit.fit()` by using `FitAddon.proposeDimensions()`. For clear-on-resize TUI runtimes, a large row drop is delayed for a short stable retry; duplicate observer events stay delayed, and only the scheduled retry may apply the same pending collapsed dimensions. If the collapse was transient, the restored size wins without any local xterm reflow, local clear, or backend resize at the collapsed rows. Small ordinary resizes and shell-like runtimes keep the existing immediate resize behavior.
