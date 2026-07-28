# Width-tagged replay — retire the ring purge

## Status

Implemented. Root-cause fix for the recurring launch/spawn width family: [#306](https://github.com/yicheng47/runner/issues/306), [#352](https://github.com/yicheng47/runner/issues/352), [#363](https://github.com/yicheng47/runner/issues/363), [#366](https://github.com/yicheng47/runner/issues/366), [#367](https://github.com/yicheng47/runner/issues/367), impls 0035/0036/0038/0039.

### The confirmed mechanism

Reproduced on a build carrying #367's merge: after restart, most tabs render garbled — two frames interleaved character-by-character in the same cells, not narrow wrapping. Clicking a tab *before* it renders leaves that tab correct. Resizing the window repairs any tab.

The drain is where it happens. `tryDrainReplay` (`RunnerTerminal.tsx:1198`) runs on activation:

```js
fit.fit();       // :1243 — resize the terminal to the PANE width
t.reset();
queued.forEach(ev => write(decodeBase64Chunk(ev.data)));   // bytes produced at the FORK width
```

The snapshot was fetched at **mount** (`:1339`) — while the pane was hidden — and parked in `pendingSnapshotRef`. So the drain fits to the pane width and then writes bytes laid out for a different width. The backend purge cannot prevent this: it fires asynchronously after the resize IPC, long after the frontend already holds the bytes client-side.

That accounts for all three observations. A hidden pane never pushes its size, so the fork width persists for as long as the agent is producing output. Activating early fits and pushes the real size before the agent paints. A window resize forces a full repaint at the correct width.

It also explains why every fork-width fix missed: each was correct but only sufficient when the fork width happened to equal the eventual pane width.

### Correction to an earlier deferral

This document was briefly marked deferred on the reasoning that the ring is empty at restart. That was wrong — the ring is empty at *process start*, but fills between resume and first tab activation, which is exactly the window the bug lives in. Same shape as the MCP-mission case, triggered by restart rather than by MCP.

### Note on scope

Decision 4 (deleting the backend purge) is severable. The rendering fix is decisions 1-3 and 6 — replay at the produced width, then reflow. The purge can be removed in the same change or left as dead weight for a follow-up; deleting it is not required to fix the symptom.

## Problem

The ring stores raw PTY bytes with no record of the width they were produced at (`session/manager/mod.rs:401-409`). Those bytes are width-coupled — absolute cursor positioning and hard wraps baked in — so replaying them into a different-width grid shreds them. Runner's answer was to detect the mismatch and destroy the history instead (`output.rs:538-546`, impl 0020).

That turned "what width should we fork at?" into a data-integrity invariant: the width chosen *before the pane exists* must exactly equal the width the pane will have, or the transcript is deleted. No component owns that invariant. It is re-decided at three `unwrap_or(DEFAULT_PTY_SIZE)` sites (`spawn.rs:233`, `:789`, `:1077`) across ~7 call paths, and it **defaults to wrong** — any caller that doesn't thread a measurement silently gets 80×24.

Hence the recurrence. Each fix taught one more caller to guess right; none removed the need to guess.

## The mechanism, already proven

`src/lib/xtermReflow.test.ts` (9 tests, green on the pinned `@xterm/xterm 6.1.0-beta.220`) establishes:

- A completed wrapped line reflows correctly when widened **and** when narrowed.
- **Deep scrollback reflows**, not just the viewport — 30 lines past the viewport all rejoin.
- Multi-run replay works: write@80 → resize 200 → write → resize 150 → write → resize 120 leaves all three runs intact.
- Resizing without awaiting `write()`'s callback matches the awaited result.
- Alt-screen excursions leave the primary scrollback beneath them untouched.
- **The unterminated line the cursor sits on does not reflow.** Deliberate and fine to rely on: that is the live frame the agent repaints on SIGWINCH.

So replaying each width-run at the width it was produced at, then resizing to the pane, reconstructs history correctly. No server-side terminal state, no VT crate, no new process.

## Key Decisions

1. **Tag ring chunks with the cols they were produced at.** `OutputEvent` gains a width. `last_pty_cols` stops being a purge gate and becomes the current-width bookkeeping that feeds the tag.
2. **Replay in width-runs.** Group consecutive same-width chunks; for each run `resize(runCols, rows)` → `write(bytes)` → await the callback → next run. Finish with a resize to the pane's real grid.
3. **Replay resizes must never reach the PTY.** Suppress the backend push for the duration of the drain — `resizeDisabledRef` (`RunnerTerminal.tsx:196`) already gates `pushSize` (`:789`) and is the mechanism to reuse. A replay resize that propagates would SIGWINCH the child to 80 and defeat the entire change.
4. **Delete the backend purge and its cols-gate**, `output.rs:538-546`. `runtime_clears_on_resize` (`:796`) goes with it if nothing else reads it.
5. **Leave the frontend's live-resize clear alone.** `runtimeClearsOnResize` in `RunnerTerminal.tsx` gates a viewport clear on *live* resize to stop reflow stacking — a different concern from replay. Do not touch it here; note whether it became redundant and leave that as a follow-up.
6. **Untagged chunks fall back to the session's fork width.** Sessions already running across the upgrade, and any chunk missing a tag, replay at the fork width rather than being dropped.
7. **0039's cached grid stays.** It becomes an optimization (fork close to the destination, fewer reflows) rather than a correctness requirement. Do not rip it out in this change.

## Goals

- A session that forks at any width and is displayed at any other keeps its full scrollback, correctly wrapped.
- Spawn width stops being load-bearing: 80×24 is acceptable everywhere.

## Non-Goals

- Server-side terminal state, a VT parser crate, or a PTY-owning process. Explicitly rejected — the emulator we already ship does the work.
- Changing spawn sites, MCP tool schemas, or 0039's cache.
- Reflowing alt-screen content.
- Touching the blank gate, the #108 wake dance, or the transitional latch beyond what decision 3 requires.

## Implementation Phases

### Phase 1 — backend tagging

- `OutputEvent` carries the width its bytes were produced at; the ring and live fanout both populate it.
- Delete the cols-gate purge (decision 4).
- Tests: chunks written across a resize carry distinct widths; a resize no longer empties the ring.

### Phase 2 — segmented replay

- Group the snapshot into width-runs and drain per decision 2, inside the existing `pendingSnapshotRef` / `tryDrainReplay` path.
- Gate backend resize pushes for the drain's duration (decision 3).
- Tests: a snapshot spanning three widths replays to the same buffer the spike's multi-run test produces; no `session_resize` is issued during a drain.

### Phase 3 — validation

- `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- `src/lib/xtermReflow.test.ts` must stay green — it is the contract this design rests on.

## Verification

Automated:

- [ ] A session forked at 80 and replayed into a 200-col pane yields unwrapped, intact scrollback.
- [ ] A resize no longer purges the ring, for every runtime.
- [ ] No PTY resize is issued during replay drain.
- [ ] Untagged chunks replay at the fork width.
- [ ] The spike's reflow tests still pass.

Manual (Jason smoke-tests, packaged build):

- [ ] Start an MCP mission, leave the slot closed for several minutes, open it — full transcript, correctly wrapped.
- [ ] Quit and relaunch with auto-resume, into a window resized since quit — scrollback intact and correctly wrapped.
- [ ] Resize the window while a claude-code pane is live — no scrollback loss, no shredded box-drawing (#306 must not return).
- [ ] Switch tabs repeatedly on a mission with several slots — no flicker through intermediate widths, no history loss (#352 must not return).
