// Shared "session is closed, here are your options" surface used by
// both RunnerChat (direct chats) and MissionWorkspace (mission slot
// PTYs). Mirrors Pencil node `vS5ce`'s bottom card. The two surfaces
// previously had separate inline implementations that drifted in
// copy and styling — consolidated here so changes land everywhere.

import { Archive, Loader2, Play, PowerOff } from "lucide-react";

import type { SessionStatus } from "../lib/types";

export interface SessionEndedOverlayProps {
  status: SessionStatus;
  /** Process exit code, if known. RunnerChat tracks it from the
   *  `session/exit` event payload; MissionWorkspace doesn't surface
   *  it today (the SessionRow shape doesn't carry it), so this is
   *  optional and the copy adjusts when it's missing. */
  exitCode?: number | null;
  /** True iff the row's `agent_session_key` is non-NULL, so resume
   *  reattaches to the same agent CLI conversation. False for shell
   *  runtimes and codex chats whose post-spawn rollout capture
   *  hasn't completed — in those cases Resume just spawns a fresh
   *  agent with no conversation history, and the copy reflects that. */
  resumable: boolean;
  /** Friendly label for the runner / slot ("@architect"). When set,
   *  the resume button reads "Resume @architect"; falls back to
   *  generic "Resume chat" / "Restart chat" wording. */
  handle?: string;
  onResume: () => void;
  /** Optional. RunnerChat surfaces an Archive option (calls
   *  session_archive); MissionWorkspace's slot pane doesn't expose
   *  one because archiving a mission slot's session row would orphan
   *  the slot — handled at the mission level instead. */
  onArchive?: () => void;
  /** Variant: "card" (default) renders as a centered card overlay
   *  matching the design; "inline" anchors to the bottom of its
   *  container, used by RunnerChat where the xterm fills the same
   *  region. */
  variant?: "card" | "inline";
  /** Override the default "Session ended" header. Used by the
   *  mission feed surface where the right phrase is "Mission paused"
   *  — same visual card, different semantics. */
  title?: string;
  /** Override the default subtitle. The status/resumable/exitCode
   *  matrix doesn't fit mission-level copy, so callers in that
   *  context pass their own line. */
  subtitle?: string;
  /** Override the resume button label. Defaults derive from
   *  `handle` + `resumable`. */
  resumeLabel?: string;
  /** Override the archive button label. Defaults to "Archive".
   *  Mission / slot callsites pass "Archive mission" so the
   *  destructive scope is unambiguous; direct chat uses the
   *  default since the surface itself implies session-scope. */
  archiveLabel?: string;
}

export function SessionEndedOverlay({
  status,
  exitCode = null,
  resumable,
  handle,
  onResume,
  onArchive,
  variant = "card",
  title,
  subtitle,
  resumeLabel,
  archiveLabel,
}: SessionEndedOverlayProps) {
  const computedSubtitle = !resumable
    ? "The PTY is closed. Resume to start a fresh agent process — there's no saved conversation to pick up from this row."
    : status === "crashed"
      ? exitCode != null
        ? `The PTY exited with code ${exitCode}. Resume to start a fresh process — the prior agent conversation is preserved.`
        : "The PTY exited unexpectedly. Resume to start a fresh process — the prior agent conversation is preserved."
      : "The PTY is closed, but the conversation is preserved. Resume to pick up where you left off.";
  const finalSubtitle = subtitle ?? computedSubtitle;
  const computedResumeLabel = handle
    ? `Resume @${handle}`
    : resumable
      ? "Resume chat"
      : "Restart chat";
  const finalResumeLabel = resumeLabel ?? computedResumeLabel;
  const finalTitle = title ?? "Chat paused";

  const card = (
    <div className="flex w-full max-w-2xl flex-col gap-3.5 rounded-xl border border-line bg-panel p-5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]">
      <div className="flex items-center gap-2.5">
        <PowerOff aria-hidden className="h-4 w-4 text-fg-3" />
        <span className="text-[15px] font-semibold text-fg">
          {finalTitle}
        </span>
      </div>
      <p className="text-[13px] leading-snug text-fg-2">{finalSubtitle}</p>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onResume}
          className="flex cursor-pointer items-center gap-2 rounded-md bg-accent px-3.5 py-2 text-[13px] font-semibold text-bg hover:bg-accent/90"
        >
          <Play aria-hidden className="h-3.5 w-3.5" />
          {finalResumeLabel}
        </button>
        {onArchive ? (
          <button
            type="button"
            onClick={onArchive}
            className="flex cursor-pointer items-center gap-2 rounded-md border border-line bg-raised px-3.5 py-2 text-[13px] text-fg hover:border-fg-3"
          >
            <Archive aria-hidden className="h-3.5 w-3.5 text-fg-2" />
            {archiveLabel ?? "Archive"}
          </button>
        ) : null}
      </div>
    </div>
  );

  if (variant === "inline") {
    // Backdrop scrim sits behind the inline card so the surface
    // visually reads as paused at a glance — without it the card
    // floats over live-looking content (xterm canvas / mission feed)
    // and the "session is closed" affordance gets lost. `inset-0` on
    // the scrim covers the whole pane; the card stays anchored to
    // the bottom-center via the existing `inset-4 … items-end`
    // wrapper. Issue #173.
    return (
      <>
        <div className="pointer-events-none absolute inset-0 z-0 bg-bg/70 backdrop-blur-sm" />
        <div className="pointer-events-none absolute inset-4 z-10 flex items-end justify-center pb-10">
          <div className="pointer-events-auto">{card}</div>
        </div>
      </>
    );
  }
  return (
    <div className="flex flex-1 min-h-0 items-center justify-center p-6">
      {card}
    </div>
  );
}

/// Centered transitional pill while a resume is in flight. Mirrors
/// Pencil node `GZhHO`. Shown overlaid on a freshly-cleared xterm
/// canvas while the agent CLI re-attaches.
export function ResumingOverlay() {
  return (
    <div className="pointer-events-none absolute inset-4 flex items-center justify-center">
      <LoadingPill label="Resuming…" />
    </div>
  );
}

/// Centered transitional pill while a fresh session is spawning —
/// the moment between the spawn RPC returning and xterm's first paint
/// over the PTY. Same cyan visual as `ResumingOverlay` so "in-flight"
/// session transitions read consistently. Pass `inline` to drop the
/// absolute positioning when the caller is already a flex container
/// (e.g. MissionWorkspace's loading branch).
export function StartingOverlay({
  label = "Starting…",
  inline = false,
}: {
  label?: string;
  inline?: boolean;
} = {}) {
  if (inline) {
    return (
      <div className="flex flex-1 items-center justify-center p-6">
        <LoadingPill label={label} />
      </div>
    );
  }
  return (
    <div className="pointer-events-none absolute inset-4 flex items-center justify-center">
      <LoadingPill label={label} />
    </div>
  );
}

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

function LoadingPill({ label }: { label: string }) {
  return (
    <div className="pointer-events-auto flex items-center gap-2.5 rounded-full border border-[#1F3D4D] bg-[#0F1E26] px-4 py-2 text-[13px] font-medium text-[#39E5FF] shadow-[0_8px_30px_rgba(0,0,0,0.5)]">
      <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
      {label}
    </div>
  );
}

/// Centered amber pill shown while an archive RPC is in flight.
/// Mirrors Pencil nodes `q3X0Ck` (mission workspace) and `FpUkw`
/// (runner chat). Geometry matches the resuming pill but the palette
/// signals a destructive transition. Pass `withScrim` for the chat
/// variant — the chat body dims behind the pill so the terminal
/// stays faintly visible.
export function ArchivingOverlay({ withScrim = false }: { withScrim?: boolean }) {
  return (
    <>
      {withScrim ? (
        <div className="pointer-events-none absolute inset-0 bg-[#15161BF2]" />
      ) : null}
      <div className="pointer-events-none absolute inset-4 flex items-center justify-center">
        <div
          className="pointer-events-auto flex h-[30px] items-center gap-2 rounded-[15px] border border-[#FFB02055] bg-[#FFB02022] px-3 font-mono text-[13px] font-semibold tracking-[0.5px] text-[#FFB020]"
        >
          <span className="h-2 w-2 animate-pulse rounded-[4px] bg-[#FFB020]" />
          Archiving…
        </div>
      </div>
    </>
  );
}
