// Shared "session is closed, here are your options" surface used by
// both RunnerChat (direct chats) and MissionWorkspace (mission slot
// PTYs). Mirrors Pencil node `vS5ce`'s bottom card. The two surfaces
// previously had separate inline implementations that drifted in
// copy and styling — consolidated here so changes land everywhere.

import { Archive, Loader2, Pause, Play } from "lucide-react";

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
   *  generic "Resume" wording. */
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
  // Short labels. The title above ("Chat paused" / "Mission paused")
  // already names the surface, so the button just needs to name the
  // action. Slot-level callsites still get a "Resume @handle" form
  // when `handle` is provided — there's no enclosing title there to
  // tell the user which slot the action targets.
  const computedResumeLabel = handle ? `Resume @${handle}` : "Resume";
  const finalResumeLabel = resumeLabel ?? computedResumeLabel;
  const finalTitle = title ?? "Chat paused";

  const card = (
    <div className="flex w-full max-w-2xl flex-col gap-3.5 rounded-xl border border-line bg-panel p-5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]">
      <div className="flex items-center gap-2.5">
        <Pause aria-hidden className="h-4 w-4 text-fg-3" />
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

function LoadingPill({ label }: { label: string }) {
  // Resume / Starting pill — info-toned ("transitional / waiting"
  // semantic). Was hardcoded to Carbon's #39E5FF + matching tints;
  // now routed through `--color-info` so the pill picks the right
  // hue per active theme (cyan in Carbon, sapphire in Codex Light,
  // sky in Catppuccin Mocha/Latte).
  return (
    <div className="pointer-events-auto flex items-center gap-2.5 rounded-full border border-info/40 bg-info/10 px-4 py-2 text-[13px] font-medium text-info shadow-[0_8px_30px_rgba(0,0,0,0.5)]">
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
  // Archiving pill — warn-toned (destructive transition). The scrim
  // + pill chrome were Carbon-only hexes (#15161B for the bg, #FFB020
  // for the amber). Now routed through `--color-bg` and `--color-warn`
  // so the dim + pill colors track the active theme on both surfaces.
  return (
    <>
      {withScrim ? (
        <div className="pointer-events-none absolute inset-0 bg-bg/95" />
      ) : null}
      <div className="pointer-events-none absolute inset-4 flex items-center justify-center">
        <div
          className="pointer-events-auto flex h-[30px] items-center gap-2 rounded-[15px] border border-warn/40 bg-warn/15 px-3 font-mono text-[13px] font-semibold tracking-[0.5px] text-warn"
        >
          <span className="h-2 w-2 animate-pulse rounded-[4px] bg-warn" />
          Archiving…
        </div>
      </div>
    </>
  );
}
