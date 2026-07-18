# 42 — Headless terminal model for durable session scrollback

> **Status: closed unshipped** (2026-07-18). Issue #306 closed as not planned — the interim symptom fix shipped as PR #308 (rows-only resize ring gate + measurable WebGL attach), and the recorded long-term direction is the Rust-native UI rewrite (#307), which dissolves the parity problem this spec's Phase 0 existed to de-risk. Archived for the failure analysis (Motivation) and the fixture-corpus / round-trip-harness design, which #307 inherits as its regression suite.

Tracking issue: https://github.com/yicheng47/runner/issues/306

## Motivation

Claude-code conversation history does not survive anything that re-runs snapshot replay. The user-visible symptom: return to a chat after a route switch, window flip, or webview reload and only the latest TUI frame is visible — scrollback above it is gone. Repeated agent-driven patch attempts have failed because the loss is architectural, not a bug in any one guard.

The loss chain has three cooperating mechanisms:

1. **Every backend resize purges the snapshot source.** `SessionManager::resize` (`src-tauri/src/session/manager/output.rs`, `purge_output_buffer_keep_modes`) drops the in-memory output ring for claude-code/codex on every resize because raw bytes emitted at old cols replay garbled into a new-width grid. The frontend's activation resize dance (`RunnerTerminal.tsx`, `forceResizeDance`) sends two resizes on every tab return, so the ring almost never holds more than the latest SIGWINCH repaint — one frame.
2. **Replay is destructive.** `tryDrainReplay` (`src/components/RunnerTerminal.tsx`) calls `term.reset()` — which destroys the entire xterm buffer including scrollback — then replays the (post-purge, single-frame) ring.
3. **The anti-stacking clear blanks a screenful.** On dims-changed resizes the frontend writes `\x1b[2J\x1b[H` before pushing geometry; xterm's ED2 blanks the viewport in place rather than scrolling it into history.

Codex is immune because its alt-screen TUI keeps the transcript inside the agent process and can always repaint; claude-code paints inline into the main screen and delegates its transcript to the emulator's scrollback, so the emulator buffer is the *only* copy. Runner keeps discarding the only copy.

Each mechanism guards against a real prior regression (frame stacking, box-drawing shredding, blank-canvas-on-return), so removing any one of them locally reintroduces an old artifact. The fix has to change where history lives, not rebalance the guards.

## Prior art

- **impl 0004 — tmux runtime.** tmux was the original session backend; `capture-pane` replay could not faithfully reproduce modern TUI redraws (stacking, issue #150). Retired.
- **PR #154** — capture-pane + reset + replay on every resize. Did not fix stacking; closed.
- **PR #157 — host-side PTY sidecar with headless emulator** (`alacritty_terminal::Term` + serializer + host-side key translation over IPC). Closed after one review pass surfaced four correctness problems: (1) two parsers had to agree across the full protocol surface, (2) the serializer had to round-trip every user-facing mode, (3) seq numbering raced across lock/IPC boundaries, (4) keys/paste needed host-side translation to honor mode bits.
- **impl 0011 — current architecture.** In-process `portable-pty`, xterm.js as the only terminal model, plus the "tiny ring buffer of recent raw bytes — pure UX patch, no emulator" that became today's `output_buffer`. The §"Why no headless emulator" decision explicitly accepted no-reattach-state.

The calculus has changed since 0011: the "pure UX patch" ring has itself grown a compensation web (purge-on-resize with a per-runtime DB probe, frontend 2J clears, the activation resize dance, synthetic alt-screen/bracketed-paste prefixes derived from regex chunk scans, overflow snapshot refetch), and the remaining user-facing cost — claude-code history loss — is the top dogfooding friction and has resisted multiple patch rounds. That is the same fragility signal that justified the 0011 pivot, now pointing back the other way.

This is also the industry-standard shape: VS Code's pty host feeds every persistent terminal's bytes into `@xterm/headless` and reattaches by replaying a `SerializeAddon` stream; tmux, mosh, WezTerm mux, and shpool all keep the terminal state machine with the PTY and treat the client as a repaintable view.

**Field validation — Orca** (`~/repos/orca`, Electron, same product category: parallel claude-code/codex orchestration) ships exactly this design in production: a daemon-side `TerminalHost` with a per-session write-only `HeadlessEmulator` (`@xterm/headless` + `SerializeAddon` + Unicode11), snapshot restore via serialized ANSI + explicit mode-rehydrate sequences, PTY lifetime decoupled from view lifetime, and an on-disk framed history log for restart-survivable replay. Their fidelity bug inventory (`src/main/daemon/headless-emulator.ts` comments) is the field-tested checklist for our Phase 0 fixtures: partial escape sequences split across chunk boundaries, Unicode/emoji width parity between mirror and renderer, SerializeAddon's relative-cursor restore bug, SGR reset before alt-screen re-enter, and query-reply double-answer races. Notably they hit all of these *with the same xterm engine on both sides*; a Rust-crate mirror has a strictly harder parity problem and must budget for it.

## Design

One sentence: keep the live path exactly as it is, add a passive in-process terminal model per session, and make `output_snapshot` serialize the model instead of replaying raw history.

- **Live path unchanged.** Raw PTY bytes → base64 → `session/output` events → xterm. xterm.js remains the hot-path renderer; the model is never in the render loop. Input path (stdin, keys, paste) unchanged.
- **`TermModel` per session.** A vt-parser + grid + scrollback instance owned by session state, fed each `RuntimeOutput::Stream` chunk in the forwarder thread (replacing `update_terminal_mode_state`'s regex scans), and resized in `SessionManager::resize` alongside the PTY so reflow tracks the real grid.
- **Snapshot = serialize.** `output_snapshot` serializes the model under the existing per-session state lock into a single synthetic chunk: full reset, mode restoration (alt-screen, bracketed paste, app cursor keys, wrap), scrollback oldest-first with SGR runs, screen, cursor position. The chunk carries `seq = output_seq` at lock time, so the frontend's `seq <= lastWrittenSeq` dedupe works unchanged.
- **Frontend replay unchanged mechanically.** `tryDrainReplay` still resets and writes the snapshot — but the snapshot now reconstructs everything, so the reset stops being lossy. `MAX_PENDING_LIVE_EVENTS` overflow refetch stays and becomes cheap (refetch = re-serialize).
- **Compensations deleted once stable:** `purge_output_buffer_keep_modes` + the `runtime_clears_on_resize` DB probe, the frontend `\x1b[2J\x1b[H` clears, the activation resize dance (a faithful snapshot removes the need to beg the agent for a repaint; the dance may survive only in the resume flow), the synthetic mode prefix, and eventually the raw ring itself.
- **All runtimes go through the model.** Codex serialization of an alt screen is just the current frame + modes (equivalent to today); shells gain reflow-correct replay. The per-runtime clear/purge switches go away.

### How this answers PR #157's four problems

1. *Two parsers must agree* — only at snapshot boundaries now, and the serializer emits a small normalized vocabulary we control (text, SGR, cursor, a fixed mode set), not arbitrary passthrough. The live hot path stays single-parser.
2. *Mode round-trip* — still required, but bounded, and half of it already exists ad hoc (the synthetic prefix); the model derives modes from a real parser instead of chunk-boundary-fragile regex scans.
3. *Seq races* — no sidecar, no IPC; serialization happens under the same session-state mutex that owns `output_seq`.
4. *Host-side key translation* — not needed; the model is output-passive and the input path is untouched.

## Scope

In scope: in-process `TermModel` + serializer, snapshot swap, compensation removal, per-session scrollback cap, fixture corpus of real claude-code/codex byte logs.

Out of scope: process-survival sidecar (0011's decision stands — agents die with the app; `--resume` covers restarts), disk persistence of terminal state across app restarts, Windows/conpty, image protocol (sixel/iTerm2) fidelity in serialization (degrade to blank cells), OSC 8 hyperlink preservation (nice-to-have, not gating).

## Implementation Phases

### Phase 0 — Fixture corpus + crate spike

Capture real byte logs (temporary forwarder tap): a long claude-code conversation including resizes, a codex session with alt-screen + the startup query handshake, a plain shell. Evaluate candidate crates against the corpus: `vt100` (ships `contents_formatted`/`state_formatted` — a built-in SerializeAddon analog), `alacritty_terminal` (battle-tested grid, serializer hand-written), `wezterm-term`.

Hard gate for candidates: **resize reflow must match xterm.js semantics** — wrapped lines tracked as wrapped and re-flowed through width changes, so scrollback serialized after a resize matches what a live xterm that watched the same stream would hold. Resize reflow is the exact failure this feature exists to fix (the ring purge existed because old-width bytes replay wrong), so a model that truncates/pads on resize is disqualified regardless of its other merits. This is the known risk for `vt100`; if it fails the gate, fall back to `alacritty_terminal`.

Second hard gate: **Unicode width parity with xterm.js** — the model must advance the cursor over emoji/ZWJ/wide characters exactly as the renderer does (xterm.js uses Unicode 11 tables in modern setups). Claude-code's status line is emoji-dense; a width mismatch accumulates cell-shifted tears in the mirror that a restore then paints back as garbage. Orca hit exactly this even with xterm-on-both-sides and fixed it by forcing matching unicode providers. Fixtures must include emoji-dense claude-code status-line output, and the round-trip harness must compare grids cell-by-cell, not line text.

Serializer detail learned from Orca: a chunk ending mid-escape leaves the partial sequence in the parser, not the grid, so a snapshot taken at that moment drops it and the next live chunk's continuation renders as literal text. The model must carry the partial-escape tail across chunk boundaries and the snapshot must ship it as a separate field the frontend writes *last* (after any reset), so the next live chunk completes it. Fixtures must include escape sequences split across chunk boundaries — the same hazard `update_terminal_mode_state`'s "boundary caveat" comment acknowledges today.

Head start for the `alacritty_terminal` path: PR #157's branch already contains an `alacritty_terminal::Term` mirror + `screen_to_ansi` serializer that compiled and passed tests. It was closed for architectural reasons (sidecar, IPC, seq races, host-side key translation) that do not apply to the passive in-process design — resurrect the serializer from that branch as the spike's starting point rather than rewriting it.

Deliverable: crate decision + fixtures checked into `src-tauri/tests/fixtures/` + a round-trip harness (feed bytes → serialize → feed serialization into a fresh model → grids must match).

### Phase 1 — Model plumbing, no behavior change

`TermModel` in session state; forwarder feeds chunks; `resize` resizes the model; `update_terminal_mode_state` replaced by model-derived mode flags (synthetic prefix now reads from the model). Ring and all existing behavior untouched. Unit tests over fixtures.

### Phase 2 — Snapshot swap

`output_snapshot` returns the serialized model chunk. Ring keeps filling but is no longer the snapshot source. Frontend untouched. Manual gate: full-history restore on route return and Cmd+R.

### Phase 3 — Remove compensations

Delete resize purge + `runtime_clears_on_resize`, frontend 2J clears + `runtimeClearsOnResize`, activation resize dance (retain for resume if the post-resume canvas needs the wake), `shouldDelayTerminalResize` large-drop deferral if it no longer earns its keep. Each removal validated against the regression it originally guarded (stacking, shredded reflow, blank canvas).

### Phase 4 — Cleanup

Drop the raw ring (`output_buffer`, `MAX_OUTPUT_BUFFER_CHUNKS`) and dead per-runtime switches; keep `output_seq` (event ordering) and `resume_watermark_seq` (pill fast-paths). Update `docs/arch/arch.md` PTY-runtime row and the `pty_runtime.rs` header comment that still points at 0011's "no headless emulator" decision.

## Verification

- Round-trip property tests on the fixture corpus: serialize→replay produces an identical grid, including after mid-stream resizes.
- Reflow tests: feed at 120 cols, resize model to 80, serialize — no shredded box-drawing, wrapped lines marked wrapped.
- Mode tests: alt-screen and bracketed-paste state correct after serialization (codex fixture), replacing the regex-scan tests.
- Perf budget: serialization of a 10k-line scrollback under ~50ms; steady-state model memory bounded by the scrollback cap.
- Manual smoke (owner: Jason): long claude-code chat → tab switch, route to Missions and back, Cmd+R, second-window flip — scroll-up shows full history in every case; window resize produces no stacking and no history loss; codex and shell panes behave as today or better.

## To be decided

- Scrollback restore cap: VS Code defaults to ~100 lines; Runner wants far more. Proposal: model cap 10k lines, serialize up to the frontend's configured xterm scrollback.
- Whether the resume flow keeps the SIGWINCH dance after the snapshot swap, or resume also becomes serialize-first.
- Crate choice (phase 0 decides against the reflow gate; `vt100` first look for the built-in serializer, `alacritty_terminal` + the PR #157 serializer as fallback).
- Whether a later phase persists serialized state to disk for restart-survivable scrollback (currently out of scope; `--resume` covers restarts). If revisited, Orca's shape is the proven one: an incremental framed append-log (length-prefixed output/resize/clear frames with a generation header) so a torn final append truncates cleanly instead of replaying half an escape sequence, replayed into the model on startup.

If the parity fallback ladder is exhausted in practice after this ships, the recorded long-term option is the Rust-native UI rewrite (#307) — one parser as both model and renderer. This spec's fixture corpus doubles as that rewrite's regression suite either way.
