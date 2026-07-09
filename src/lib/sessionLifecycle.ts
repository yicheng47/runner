// Session lifecycle helpers — small pure functions that the chat and
// mission panes use to gate transitional overlays (the cyan "Starting"
// pill, "Resuming" pill, etc.). Lives in `lib/` instead of next to the
// overlays in `components/SessionEndedOverlay.tsx` because mixing
// component exports with plain functions in one module breaks Vite's
// React Fast Refresh — the file falls back to a full reload on any
// edit. Splitting them out gives both clean HMR behavior and a more
// honest home for these as logic primitives, not UI.

import type { SessionOutputEvent } from "./types";

/// True iff `startedAt` is within the last few seconds — the only
/// case where the StartingOverlay pill should fire. Switching tabs
/// to a chat that's been running for an hour, or reopening a mission
/// the day after `mission_start`, must NOT replay the boot pill; the
/// agent CLI inside the PTY is already painted and the user just
/// wants the live terminal.
///
/// Window is intentionally short (a couple of seconds): spawn → IPC
/// round-trip → React Router navigation is well under a second in
/// practice, and any session older than that has already had a
/// chance to paint its first frame.
export function isFreshSpawn(startedAt: string | null | undefined): boolean {
  if (!startedAt) return false;
  const ts = Date.parse(startedAt);
  if (!Number.isFinite(ts)) return false;
  return Date.now() - ts < 3_000;
}

/// Detects whether a `session/output` chunk contains an escape
/// sequence that signals the agent's TUI is initialized and ready
/// to receive input. Used by the starting-pill effects to clear on
/// TUI init rather than waiting for the output stream to go idle.
///
/// Why: the first-turn prompt is auto-delivered at spawn (via
/// positional argv or paste), so claude-code's / codex's boot
/// output flows continuously into first-turn processing into the
/// first reply — no 400ms quiet window in between. Without an
/// explicit "ready" signal the pill stays visible until the agent
/// finishes replying, which can be many seconds.
///
/// Signals we look for, in priority order:
///   - `\x1b[?2004h` — enable bracketed paste mode. Emitted very
///     early by claude-code, codex, and most modern interactive
///     CLIs (the moment the TUI is wired up to accept input). This
///     is the strongest "ready for input" indicator we get
///     without parsing app-specific output, and the one this
///     codebase relied on after empirical capture of claude-code
///     and codex startup bytes (issue #171).
///   - `\x1b[?1049h` — modern alt-screen enter. Used by
///     full-screen TUIs (vim, htop, etc.); claude-code and codex
///     do NOT emit this — they're main-screen redraw-in-place —
///     but the check is cheap and covers any agent that does.
///   - `\x1b[?47h` — legacy alt-screen enter for older TUIs.
///
/// The data field arrives base64-encoded from the Rust side (see
/// `OutputEvent` in `src-tauri/src/session/manager.rs`); we decode
/// to a binary string and substring-search. Theoretical risk: the
/// escape spans a chunk boundary and we miss it on the split-chunk
/// frame — caller still has the idle fallback, so worst-case the
/// pill takes the old path. Not worth a rolling tail buffer.
export function chunkIndicatesTuiReady(base64: string): boolean {
  let bytes: string;
  try {
    bytes = atob(base64);
  } catch {
    return false;
  }
  return (
    bytes.includes("\x1b[?2004h") ||
    bytes.includes("\x1b[?1049h") ||
    bytes.includes("\x1b[?47h")
  );
}

/// Watermark-aware variant of the snapshot fast-path check. Since
/// impl 0024 a claude-code resume keeps the output ring, so a
/// `session_output_snapshot` taken during the resume window still
/// contains the *old* PTY's chunks — including its pre-stop
/// `\x1b[?2004h`. Only chunks with `seq` above the resume watermark
/// (`api.session.replayWatermark`) can come from the new PTY, so
/// only those may clear a starting/resuming pill. Watermark 0 (fresh
/// spawn, never-resumed session) degenerates to "any chunk counts".
export function snapshotIndicatesTuiReady(
  events: SessionOutputEvent[],
  watermark: number,
): boolean {
  return events.some(
    (ev) => ev.seq > watermark && chunkIndicatesTuiReady(ev.data),
  );
}
