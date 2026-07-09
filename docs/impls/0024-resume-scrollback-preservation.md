# Preserve claude-code scrollback across session resume

## Status

Planned. Grew out of the investigation on issue #267 (Claude Code output history not accessible from Runner). This impl fixes the in-app half of that report — resume discarding scrollback the process still had. The durable half (history across app restart, read from claude-code's JSONL transcript) is a separate feature and stays tracked by #267; this change does not close it.

## Problem

Resuming a claude-code session throws away all pre-resume terminal history, even though nothing required losing it.

`SessionManager::resume` calls `purge_output_buffer` up front (`src-tauri/src/session/manager/spawn.rs:932`), wiping the session's in-memory output ring. The mounted xterm instance still shows the old content (the resume overlay is only `opacity-0` — `src/components/ChatPaneGroup.tsx:323`), so the loss is invisible at first. It materializes on the next remount (route switch, webview reload): `RunnerTerminal`'s replay path does `term.reset()` and re-feeds the ring (`src/components/RunnerTerminal.tsx:1005`), which now contains only post-resume bytes.

What the user gets after that is whatever `claude --resume` repaints on its own — measured empirically at roughly the **last 4 turns** of the conversation, regardless of session length. A real terminal (Ghostty, iTerm) resuming the same session keeps the entire prior scrollback above the resume banner and appends the tail repaint below it. Runner deviates from that baseline for no user-visible benefit: it deletes history a terminal emulator would keep.

Why the purge exists today (contracts documented at the `spawn.rs` call site):

1. **TUI-ready race.** The starting/resuming pill effects call `session_output_snapshot` to catch a `\x1b[?2004h` (bracketed paste = TUI ready) that fired before their live listener attached (`src/pages/RunnerChat.tsx:731`, `src/pages/MissionWorkspace.tsx:1423`). If the ring still held the *old* PTY's chunks — which include the pre-stop `\x1b[?2004h` — the snapshot check would clear the pill before the new PTY exists. Purging at the top of `resume()` closes that window by brute force.
2. **Seq continuity.** `purge_output_buffer` retains the seq counter so the new PTY's first chunk continues at `last + 1` and the frontend's `seq <= lastWrittenSeq` filter doesn't drop it.
3. **No stacked frames on remount replay.** For full-frame-repaint TUIs, replaying the old frame under the new one stacks garbled content in scrollback (the artifact class from impls 0009/0011/0020).

Contract 3 is real for codex but wrong for claude-code: codex repaints the whole frame (and re-renders a large conversation tail on resume anyway — measured ~77 turns), while claude-code paints inline into the main screen, where "old content, then resume banner, then tail repaint" is exactly what a physical terminal shows. Contracts 1 and 2 can both be satisfied without deleting anything: 2 already is (the counter survives either way), and 1 needs a *boundary marker*, not an empty buffer.

## Key Decisions

1. **Keep the ring across resume for claude-code only.** Codex keeps today's purge: its full-frame repaint over retained scrollback is the stacking artifact the purge was added for, and its own resume replay already restores a deep tail. Shells and future runtimes (opencode, feature 30) keep today's behavior for scope; extending them later is a one-line gate change. Gate via a `runtime_purges_on_resume(session_id, pool)` helper mirroring `runtime_clears_on_resize` (`src-tauri/src/session/manager/output.rs:419`) — same COALESCE join, returns `true` for `codex`, `false` for `claude-code`, callable right after the row snapshot loads so the purge/skip decision still happens at the top of `resume()`, before any long-running step.
2. **A seq watermark replaces "clean buffer" as the pill contract.** At the same top-of-`resume()` point, record `resume_watermark_seq = output_seq` in `SessionState` (all runtimes, uniformly — for codex it's equal to the post-purge floor, so filtering is a no-op). The pill effects' snapshot fast-path only honors TUI-ready escapes in events with `seq > watermark`. Old chunks stay replayable for the terminal but can no longer clear the overlay early.
3. **Expose the watermark via a dedicated read command,** `session_replay_watermark(session_id) -> u64`, rather than changing `session_output_snapshot`'s return shape (which `RunnerTerminal` replay consumes as a bare array) or `session_resume`'s (a resume can be triggered from another window — impl 0018 — so the pill can't rely on the resume RPC's response reaching it). Fresh spawns report 0, so the filter is inert outside resume flows.
4. **Do not reset the terminal-mode flags on the keep path.** `purge_output_buffer` resets `alt_screen_on` / `bracketed_paste_on` because after a purge no escape bytes remain to justify a synthetic snapshot prefix. When the buffer is kept, the old chunks carry their own mode escapes and `update_terminal_mode_state` keeps deriving state from the live stream; the seq=0 synthetic prefix stays correct for the evicted-escape case it was built for. Worst case is a redundant `\x1b[?2004h` replayed from both the prefix and a surviving chunk — harmless.
5. **Accept the double-tail and stale-width artifacts.** The last ~4 turns will appear twice after a resume (once in kept scrollback, once in claude's repaint) — identical to resuming in Ghostty; not a bug. Kept bytes were emitted at the old grid width; a later remount replays them into the current grid, so lines wrap at the recorded width. For claude-code's inline text this reads like ordinary terminal reflow; it is bounded anyway because live resizes still purge the ring for claude-code (`resize` → `purge_output_buffer_keep_modes`, unchanged by this impl), so the kept segment never spans a width change that happened while the session was running.
6. **No frontend rendering changes.** In-place resume already preserves the mounted xterm buffer under the `opacity-0` overlay; this impl makes the backend ring agree with it so remounts stop losing what the screen already showed. `ResumeSettleTracker` (`src/pages/RunnerChat.tsx`) listens to live events only — live events during a resume window can only come from the new PTY (resume is refused while the row is running) — so it needs no watermark. The stale `clearVersion` comment block at `src/pages/RunnerChat.tsx:1120` gets rewritten to describe the real mechanism.

## Goals

- Resuming a claude-code session (direct chat or mission slot) keeps all pre-resume output in the ring: scrolling up after resume shows the prior conversation, and it survives route switches / remounts for the lifetime of the app process, matching what a terminal emulator would keep.
- The resuming/starting pills still clear only on the *new* PTY's ready signal or idle heuristic — never on stale pre-stop bytes.
- Seq continuity, codex resume behavior, resize purge behavior, and archive/delete purge behavior are unchanged.

## Non-Goals

- History across app restart. The ring stays process-local and bounded (arch: `docs/arch/arch.md` §scrollback); after a restart the only full record is claude-code's JSONL under `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. Surfacing that is #267's transcript-viewer feature, not this impl.
- Persisting PTY bytes to disk, raising `MAX_OUTPUT_BUFFER_CHUNKS`, or any change to the bounded-ring architecture.
- Keeping scrollback across resume for codex, shells, or other runtimes.
- Deduplicating claude's repainted tail against kept scrollback.

## Implementation Phases

### Phase 1 — backend: gated purge + watermark (`src-tauri/src/session/manager/`)

- `mod.rs`: add `resume_watermark_seq: u64` to `SessionState` (alongside `output_seq`).
- `output.rs`: add `runtime_purges_on_resume(session_id, pool) -> bool` next to `runtime_clears_on_resize` (same query; matches `Some("codex")`; DB miss → `true`, i.e. fail toward today's purge). Add a `set_resume_watermark(session_id)` + `replay_watermark(session_id) -> u64` pair on `SessionManager`.
- `spawn.rs` `resume()` (~line 929): at the existing purge point, always set the watermark from the current `output_seq`; call `purge_output_buffer` only when `runtime_purges_on_resume` says so. Rewrite the call-site comment: contract 1 is now carried by the watermark, contract 2 by the untouched counter.
- Tests (`tests.rs`): claude-code resume keeps prior chunks and new output continues seq monotonically through a snapshot; codex resume still purges; watermark equals the pre-resume max seq for both; fresh spawn reports watermark 0; archive purge (`purge_session_buffers`) resets the watermark with the rest of the state.

### Phase 2 — command + api surface

- `src-tauri/src/commands/session.rs`: `session_replay_watermark(session_id) -> u64`; register in the handler list.
- `src/lib/api.ts`: `api.session.replayWatermark(sessionId)` beside `outputSnapshot` (~line 206).

### Phase 3 — frontend: watermark-aware pill fast-paths

- `src/lib/sessionLifecycle.ts`: add `snapshotIndicatesTuiReady(events: OutputEvent[], watermark: number)` — `events.some((ev) => ev.seq > watermark && chunkIndicatesTuiReady(ev.data))` — so the filter lives in one tested helper.
- `src/pages/RunnerChat.tsx` (~731) and `src/pages/MissionWorkspace.tsx` (~1423): fetch `Promise.all([outputSnapshot, replayWatermark])` in the snapshot fast-path and use the helper. The mission path is the one that actually needs it (a resumed slot re-enters the pill via `isFreshSpawn(started_at)` with old bytes now retained); the RunnerChat `starting` path gets it for uniformity (fresh rows report 0).
- `src/pages/RunnerChat.tsx:1120`: fix the stale resume-sequence comment (`clearVersion` no longer exists; the backend no longer drops the buffer for claude-code).
- Unit tests (vitest) for the helper: below-watermark ready escape ignored; above-watermark honored; watermark 0 degenerates to today's behavior.

### Phase 4 — docs

- `docs/arch/arch.md` scrollback section (~407): note that resume preserves the ring for claude-code (bounded, process-local as before) and why codex still purges.
- Doc comments on `purge_output_buffer` (`output.rs:333`) — no longer "used by resume" unconditionally.

### Phase 5 — verify

- `cargo fmt && cargo clippy && cargo test --workspace`; `pnpm exec tsc --noEmit && pnpm run lint`.
- Manual smoke (user-run): claude-code chat with a dozen turns → Stop → Resume → scroll up: pre-resume history present above the resume repaint; switch route away/back → still present; quit + relaunch → gone as before (expected; see Non-Goals); codex slot Resume → unchanged (fresh repaint, no stacked frames); mission Resume all with mixed claude/codex slots → pills clear on new-PTY readiness, not instantly.

## Relevant Code

- `src-tauri/src/session/manager/spawn.rs:929` — the purge call in `resume()` and its two-contract comment.
- `src-tauri/src/session/manager/output.rs:263-357` — `output_snapshot`, `purge_session_buffers`, `purge_output_buffer`, `purge_output_buffer_keep_modes`; `runtime_clears_on_resize` at 419 (pattern for the new gate).
- `src-tauri/src/session/manager/mod.rs:50` — `MAX_OUTPUT_BUFFER_CHUNKS`; `SessionState` fields (~447).
- `src-tauri/src/commands/session.rs:462` — `session_resume`; new command lands beside it.
- `src/pages/RunnerChat.tsx:697-770, 1120-1170, ResumeSettleTracker` — starting pill, resume sequence, settle tracker.
- `src/pages/MissionWorkspace.tsx:600-667, 1385-1463` — mission resume-all dims fallback, slot starting pill.
- `src/components/ChatPaneGroup.tsx:314-361` — transitional `opacity-0` pane (unchanged, but the mechanism this impl aligns with).
- `src/components/RunnerTerminal.tsx:953-1105` — replay drain (`reset()` + snapshot re-feed) that turns the ring purge into visible loss.
- `src/lib/sessionLifecycle.ts:60` — `chunkIndicatesTuiReady`.

## Open Questions

- **Extend the keep path to shells?** A resumed shell has no repaint at all, so keeping its scrollback is strictly closer to real-terminal behavior. Left out only for scope; flipping the gate later is trivial.
- **Ring budget sharing.** Old and new bytes now share the 4096-chunk ring, so a long post-resume session evicts the kept history first. That is the intended bounded behavior, but if it feels too tight in practice, bumping the cap (or capping kept-history chunks at resume time) is a follow-up knob, not part of this impl.

## References

- Issue #267 — Claude Code output history not accessible from Runner (parent problem; durable JSONL surface remains open there).
- Measurements (2026-07-09, claude-code 2.1.205 / codex-cli 0.143.0): `claude --resume` repaints only the last ~4 turns of a 20-turn session; `codex resume` repaints the full history of a short session and a ~77-turn tail of a 169-turn session. Neither enters the alternate screen.
- impl [0009](archive/0009-terminal-alt-screen-reattach.md), [0011](archive/0011-pty-host-terminal-runtime.md), [0020](0020-direct-chat-split-view.md) — the repaint/stacking artifact history behind the purge-on-resume and clear-on-resize policies.
- `docs/features/21-resume-cwd-sessions.md:25` — the original (claude-optimistic) assumption that the resumed agent repaints its own history.
