// Shared header buttons for the runner-chat / mission-workspace
// session controls. Both surfaces stack the same Resume / Stop /
// Resuming / Back affordances on the right side of their headers;
// previously each page styled the buttons inline, which let drift
// creep in (RunnerChat hard-coded Carbon-only hex shades that read
// as heavy dark blocks against the Codex Light header). These
// components route every style through semantic tokens — `accent`
// for Resume, `info` for Resuming, neutral chrome for Stop / Back —
// so the same JSX paints correctly in both themes.
//
// Sizing matches MissionWorkspace's existing pill (px-2.5 py-1
// text-[11px] font-semibold) so this is a non-visual change for
// mission and a "now matches mission" change for chat.

import type { ComponentType, ReactNode } from "react";
import { Loader2, Play, Square } from "lucide-react";

const PILL_BASE =
  "inline-flex cursor-pointer items-center gap-1.5 rounded-md border px-2.5 py-1 text-[11px] font-semibold transition-colors";

export interface SessionControlProps {
  onClick?: () => void;
  title?: string;
  /** Optional override label. Defaults to "Resume" / "Stop" / etc. */
  children?: ReactNode;
}

// Resume — accent-colored "primary action" pill. Same shape on chat
// and mission headers so the affordance is instantly recognizable
// from either page. Always carries the Play icon; the icon is the
// affordance's visual anchor and dropping it on one surface makes
// the two pages look inconsistent.
export function ResumeButton({
  onClick,
  title,
  children,
}: SessionControlProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={`${PILL_BASE} border-accent/40 bg-accent/10 text-accent hover:border-accent`}
    >
      <Play aria-hidden className="h-3 w-3" />
      {children ?? "Resume"}
    </button>
  );
}

// Resuming — info-colored disabled pill with spinner. Used while
// `session_resume` is in flight, before the new PTY paints its first
// chunk. Cyan reads as "transitional / waiting" without claiming a
// failure or a success.
export function ResumingButton({
  title,
  children,
}: Omit<SessionControlProps, "onClick">) {
  return (
    <button
      type="button"
      disabled
      title={title}
      className={`${PILL_BASE} cursor-not-allowed border-info/40 bg-info/10 text-info`}
    >
      <Loader2 aria-hidden className="h-3 w-3 animate-spin text-info" />
      {children ?? "Resuming…"}
    </button>
  );
}

// Stop — neutral chrome pill. The Square icon is red (danger) so the
// destructive flavor is clear, but the pill stays neutral so it
// doesn't compete with Resume for visual primacy.
export function StopButton({
  onClick,
  title,
  children,
  iconTone = "danger",
}: SessionControlProps & { iconTone?: "danger" | "fg" }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={`${PILL_BASE} border-line bg-raised text-fg hover:border-line-strong`}
    >
      <Square
        aria-hidden
        className={`h-3 w-3 ${iconTone === "danger" ? "text-danger" : "text-fg"}`}
      />
      {children ?? "Stop"}
    </button>
  );
}

// Back — neutral chrome pill without an icon. Used as the escape
// hatch for archived rows or non-resumable sessions ("Back to
// runner"). Same shape as Stop so the buttons align visually when
// they share a row.
export function BackButton({
  onClick,
  title,
  children,
  icon: Icon,
}: SessionControlProps & {
  icon?: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={`${PILL_BASE} border-line bg-raised text-fg hover:border-line-strong`}
    >
      {Icon ? <Icon aria-hidden className="h-3 w-3 text-fg-2" /> : null}
      {children ?? "Back to runner"}
    </button>
  );
}
