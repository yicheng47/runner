# In-place resume seam + PTY width hardening

## Status

Planned. Tracking issue [#344](https://github.com/yicheng47/runner/issues/344) plus the width-divergence findings from the 2026-07-25 rendering investigation. Builds directly on impl [0024](archive/0024-resume-scrollback-preservation.md) (per-runtime ring policy) and reuses its vocabulary: ring as single source of truth, synthetic chunks, `runtime_purges_on_resume` gate.

## Problem

Three defects, one family: what the agent emitted under one terminal state (grid content, emulator modes, width) is later viewed under another, and nothing reconciles the two.

1. **In-place resume streams into the stale grid (#344).** Stopping a session intentionally keeps its final frame visible. But resuming without leaving the tab streams the new PTY into that dead grid: `resumeSession` (`src/pages/RunnerChat.tsx:1183`; mission slots use the same `session_resume` path) never resets anything — the only `term.reset()` lives in the tab-switch/snapshot replay path (`src/components/RunnerTerminal.tsx:1120`). The backend half IS policy-aware (purge-runtimes purge the ring and reset tracked modes, `output.rs:610-622`; claude-code keeps its ring, `output.rs:725-741`), but the live grid mirrors neither policy. Consequences: (a) live/replay divergence for purge-runtimes — the live grid stacks dead frame + new frame while the purged ring holds only the new bytes, so the first later replay silently shows a different scrollback; (b) terminal-mode leakage for all runtimes — the dead session's bracketed paste (`?2004h`), mouse reporting, and SGR attributes survive in the live emulator until the resumed process happens to rewrite them; (c) dirty seam for keep-runtimes — the resume banner lands at the dead cursor position with stale attributes, no newline guarantee.
2. **Unsized spawns default to 80×24.** `pty_runtime.rs:121` falls back to `(80, 24)` whenever the caller passes no dims. Resume callers measure the mounted pane (`RunnerTerminalHandle.measure`), but the pane is frequently unmeasurable at resume time: hidden persistent layer, unmounted route, resume triggered from another window (impl 0018), or app-relaunch resume before layout settles. Bytes emitted in that window are hard-wrapped at 80 cols; the later fit resizes the PTY but cannot unwrap them. This is the measured Δ≈cols−80 miswrap fingerprint (`RunnerTerminal.tsx:133-139`, `#resume-pty-size-mismatch`).
3. **Hidden persistent surfaces defer geometry.** `PersistentSurfaces` hides the inactive layer with `display:none` (`src/components/PersistentSurfaces.tsx:68,79`), which makes containers unmeasurable, so `refitAndPush` is gated off while hidden (`RunnerTerminal.tsx:767-772`). Geometry changes made from the other surface (left sidebar, window size, zoom) reach the hidden pane's PTY only on activation. Everything the agent emitted in between carries the stale width — agent-side hard wraps that no reflow can repair.

Explicitly NOT part of this problem: the duplicated-block scrollback artifact. That is upstream claude-code's inline renderer re-emitting the in-progress conversation during multi-tool turns (anthropics/claude-code#52866; the 2.1.121 partial fix does not cover xterm.js hosts). Runner renders those bytes faithfully. User-side workaround is `/tui fullscreen`.

## Key Decisions

1. **Fix resume grid state through the byte path, not frontend choreography.** Both per-runtime policies materialize as synthetic chunks written into the ring through the normal output ingest path — real seq, `session/output` event fanout, `update_terminal_mode_state` derivation — exactly like the seq-0 synthetic mode prefix the snapshot path already prepends (`output.rs:529-556`), but at resume time. Rationale: a resume can be triggered from a window that is not displaying the pane (impl 0018; see 0024 decision 3), and the pane may also be a hidden-but-live persistent layer. Only the byte path reaches every mounted view and every future replay identically. No new frontend invariants.
2. **Purge-runtimes (codex, shells): seed the purged ring with a synthetic full-reset chunk.** After the existing `purge_output_buffer`, append one chunk that leaves any live grid indistinguishable from a fresh terminal: scrollback gone, modes off, SGR clean, cursor home. Candidate bytes: `\x1bc` (RIS) if xterm.js RIS clears scrollback; otherwise compose `\x1b[3J\x1b[2J\x1b[H\x1b[0m` plus explicit mode-offs. The coder verifies xterm.js behavior and picks; the requirement is behavioral, not a byte spec. Replay is unaffected (replay already `term.reset()`s before re-feed; the chunk is idempotent there).
3. **Keep-runtimes (claude-code): append a synthetic seam chunk, scrollback intact.** `\x1b[0m` + `\x1b[?2004l` + mouse-reporting off (`\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l`) + `\r\n` — the inverse of the synthetic snapshot prefix. The resume banner starts on a fresh line with clean attributes; kept scrollback above it is untouched. This is the "physical terminal continuity" model 0024 chose, minus the unsanitized seam.
4. **Seam chunks ride the standard ingest path so tracking and pills stay honest for free.** Mode tracking flips from the seam bytes themselves (no manual flag writes — 0024 decision 4's reasoning). The resume watermark is set before the seam is appended; the seam contains only mode-OFF escapes, so `chunkIndicatesTuiReady` can never fire on it and the starting/resuming pills still wait for the new PTY.
5. **Sized-spawn fallback chain: explicit dims → last-applied dims → 80×24.** Persist the last applied cols/rows per session (two INTEGER columns on `sessions`, written on every applied resize). `spawn`/`resume` use them whenever the caller passes `None`. 80×24 remains only for a truly-first spawn that has never been sized. Persisting in SQLite (not memory) means app-relaunch resume forks the child at the right width before any frontend layout exists — the exact window behind the cols−80 miswrap.
6. **Hidden persistent layers stay laid out and keep pushing geometry.** `PersistentSurfaces` switches its hidden layer from `display:none` to invisible-but-measurable: absolutely stacked over the content area, `visibility:hidden`, `pointer-events:none`, `aria-hidden`. `RunnerTerminal` splits its single `active` gate into two concerns: geometry (fit, `pushSize`, per-runtime resize-clear) follows measurability; rendering, focus, WebGL retention, wake-dance, and shortcut handling stay keyed to visibility exactly as today. With dims always current, the activation-time deferred resize dedupes to a no-op, which also retires the "paints stale until a real grid change" window noted at `RunnerTerminal.tsx:296`.
7. **Accepted consequence of decision 6:** live resizes purge the claude-code ring (`resize` → `purge_output_buffer_keep_modes`, 0024 decision 5), and hidden panes will now resize live, so ring purges can happen while hidden. The mounted xterm buffer retains full scrollback either way (keep-alive); only post-restart/remount replays hold less history — the same tradeoff already accepted for visible resizes.

## Goals

- Stop → in-place Resume with no navigation: a codex pane comes back to a clean grid identical to what a later remount replay shows; a claude-code pane keeps its scrollback with the banner on a fresh clean line. Pasting immediately after resume behaves as plain input (no inherited bracketed paste).
- The same session renders identically live and via any later replay after a resume — divergence class (a) is gone.
- A resume or spawn with no measurable pane (hidden layer, other window, app relaunch) forks the child at the session's last-known dims, not 80×24.
- A geometry change made while on the other surface reaches the hidden mounted pane's PTY at the time of the change; returning to it needs no activation resize and shows output wrapped at the width it was actually emitted for.

## Non-Goals

- The upstream claude-code duplicated-block artifact (see Problem; not fixable in a bytes-as-truth host).
- Per-tab right-panel state. Global panel state (`runner.chat.panel.open`/`.width`, `runner.mission.rail.open`) is intended chrome behavior.
- Repairing history already hard-wrapped at an old width. Impossible once the agent split the lines itself.
- Extending keep-ring resume to shells (0024's open question stays open) or changing ring bounds.
- Multi-window simultaneous same-session viewing (PTY-daemon territory, deferred with tripwires).

## Implementation Phases

### Phase 1 — backend: synthetic resume chunks (`src-tauri/src/session/manager/`)

- `output.rs`: a helper that appends a synthetic chunk through the normal ingest path (seq assignment, event fanout, mode-state derivation). Constants for the purge-reset bytes and the keep-seam bytes next to the existing synthetic prefix builder (`:529-556`).
- `spawn.rs` `resume()` (~`:932`, the 0024 purge/watermark point): after watermark + purge decision, append the policy's chunk — reset chunk on the purge path, seam chunk on the keep path — before the child forks.
- Tests (`tests.rs`): purge-runtime ring after resume = reset chunk then child bytes; keep-runtime ring = prior chunks, seam, child bytes, seq monotonic; mode tracking reports paste/mouse off immediately after the seam; watermark excludes the seam from pill fast-paths; archive/delete purge behavior unchanged.

### Phase 2 — backend: dims persistence + sized fallback

- `db.rs`: migration adding `last_cols`/`last_rows` (nullable INTEGER) to `sessions`.
- Resize path (`session_resize` handling): persist applied dims. `spawn`/`resume`: fallback chain from decision 5 where `initial_size` is `None` (`pty_runtime.rs:121` consumes the resolved value; keep the resolution in the manager, not the runtime).
- Respect the existing mission resume-all dims fallback (`src/pages/MissionWorkspace.tsx:600-667`) — frontend-measured dims still win when available.
- Tests: resume with `None` dims uses persisted dims; first-ever spawn still 80×24; persisted dims survive a manager restart (DB round-trip).

### Phase 3 — frontend: measurable/active split

- `src/components/PersistentSurfaces.tsx`: hidden layer becomes invisible-but-measurable (decision 6). The visible layer's `contents` wrapper and the `visible` prop semantics (window subjects, shortcut listeners) are unchanged.
- `src/components/RunnerTerminal.tsx`: `refitAndPush` gates on measurability (container rect > 0) instead of `activeRef`; WebGL release, focus, wake-dance, and the transitional latch stay on the existing `active`/`disabled` gating. The per-runtime `\x1b[2J\x1b[H` pre-clear on resize applies to hidden panes too (their buffer keeps parsing live output, so it stays consistent).
- Audit the activation effect: with dims current, activation should reduce to focus + WebGL restore + replay drain for never-activated panes. Do not remove the drain conditions (`RunnerTerminal.tsx:1062-1077`) — never-activated panes still park replay until first activation.
- `pnpm exec tsc --noEmit`, `pnpm run lint`, vitest for any extracted helpers.

### Phase 4 — docs + validation

- `docs/arch/arch.md` scrollback/resume section: document the synthetic seam chunks and the dims fallback chain.
- Manual smoke list (user-run): claude chat stop → in-place resume → clean seam line, paste plain, scroll-up history intact; codex slot stop → in-place resume → clean grid, tab away/back → identical; quit app mid-session → relaunch → resume from the sessions list before opening the chat → open it → no 80-col wrapped segment; collapse sidebar while on mission surface → return to chat → no resize flash, correct wrapping.

## Relevant Code

- `src-tauri/src/session/manager/spawn.rs:932` — resume purge/watermark point (0024).
- `src-tauri/src/session/manager/output.rs:529-556` — synthetic mode prefix (pattern for the seam builder); `:610-622` purge + tracked-mode reset; `:725-741` `runtime_purges_on_resume`.
- `src-tauri/src/session/pty_runtime.rs:121` — `initial_size.unwrap_or((80, 24))`.
- `src-tauri/src/db.rs` — schema + migrations.
- `src/components/PersistentSurfaces.tsx:67-88` — hidden-layer wrappers.
- `src/components/RunnerTerminal.tsx:703-742` — `pushSize` + per-runtime pre-clear; `:767-772` hidden-pane gate; `:1062-1077` replay drain conditions; `:1223+` transitional latch (must survive Phase 3 untouched).
- `src/pages/RunnerChat.tsx:1183` — `resumeSession`; `src/pages/MissionWorkspace.tsx:600-667` — resume-all dims fallback.

## Open Questions

- RIS (`\x1bc`) vs composed reset bytes for the purge chunk — decided by the coder against actual xterm.js 6.1.0-beta behavior (does RIS clear scrollback?). Behavioral requirement is fixed either way.
- Whether the keep-seam should also fire on `resize`-triggered ring purges for claude-code. Out of scope here; today's behavior stands.
- Whether a future keep-seam should conditionally exit alt-screen when tracked state says it is active. Impl 0032 follows decision 3's fixed seam bytes and does not emit `\x1b[?1049l`; adding it unconditionally is unsafe because xterm.js restores the saved cursor even when already on the main buffer, so any follow-up must be state-aware.

## References

- Issue #344 — in-place resume streams into the stale xterm grid (this impl closes it).
- Impl 0024 — per-runtime ring policy, watermark, synthetic prefix vocabulary.
- anthropics/claude-code#52866 — upstream duplicated-block renderer bug (explicitly out of scope).
- Investigation notes 2026-07-25: cols−80 miswrap fingerprint; resize-source inventory (sidebar, window, global panel state); hidden-pane deferred resize confirmed at `RunnerTerminal.tsx:767-772`.
