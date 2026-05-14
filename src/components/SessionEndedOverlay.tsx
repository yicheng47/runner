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
  const finalTitle = title ?? "Session ended";

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
            Archive
          </button>
        ) : null}
      </div>
    </div>
  );

  if (variant === "inline") {
    return (
      <div className="pointer-events-none absolute inset-4 flex items-end justify-center pb-10">
        <div className="pointer-events-auto">{card}</div>
      </div>
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
      <div className="pointer-events-auto flex items-center gap-2.5 rounded-full border border-[#1F3D4D] bg-[#0F1E26] px-4 py-2 text-[13px] font-medium text-[#39E5FF] shadow-[0_8px_30px_rgba(0,0,0,0.5)]">
        <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
        Resuming…
      </div>
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
