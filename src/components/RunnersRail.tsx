// Right-hand rail in the mission workspace — one card per runner in the
// roster, showing PTY status (running / stopped / crashed), last
// `runner_status` (busy / idle), the LEAD badge, and an "open pty" link
// that switches the center column to that runner's terminal tab.

import type { SessionRow } from "../lib/api";

interface RunnersRailProps {
  sessions: SessionRow[];
  /** Selected tab in the workspace. `null` means the feed tab is active. */
  selectedSessionId: string | null;
  /** Latest `runner_status` (busy/idle) per handle, projected from the
   *  event feed by the workspace. Missing handles render as no badge. */
  status: Record<string, "busy" | "idle">;
  /** Stable lead handle for the mission, used to badge the right card. */
  leadHandle: string;
  onOpenPty: (sessionId: string) => void;
}

export function RunnersRail({
  sessions,
  selectedSessionId,
  status,
  leadHandle,
  onOpenPty,
}: RunnersRailProps) {
  return (
    <aside className="flex w-72 shrink-0 flex-col gap-3 border-l border-line bg-panel p-5">
      <div className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
        Runners
      </div>
      {sessions.length === 0 ? (
        <p className="text-xs text-fg-3">No runners spawned.</p>
      ) : (
        sessions.map((s) => {
          const isLead = s.handle === leadHandle;
          const ptyStatus = s.status;
          const dotClass =
            ptyStatus === "running"
              ? "bg-accent"
              : ptyStatus === "crashed"
                ? "bg-danger"
                : "bg-fg-3";
          const runnerStatus = status[s.handle];
          const subtitle =
            ptyStatus === "running"
              ? runnerStatus
                ? runnerStatus
                : "running"
              : ptyStatus;
          const selected = selectedSessionId === s.id;
          return (
            <div
              key={s.id}
              className={`flex flex-col gap-1.5 rounded-md border bg-bg p-3 transition-colors ${
                selected
                  ? "border-accent/60"
                  : "border-line hover:border-line-strong"
              }`}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-1.5 min-w-0">
                  <span
                    className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${dotClass}`}
                    title={ptyStatus}
                  />
                  <span className="truncate font-mono text-[13px] font-semibold text-fg">
                    @{s.handle}
                  </span>
                  {isLead ? (
                    <span className="rounded bg-warn/20 px-1.5 py-px text-[9px] font-bold uppercase tracking-wide text-warn">
                      Lead
                    </span>
                  ) : null}
                </div>
                <button
                  type="button"
                  onClick={() => onOpenPty(s.id)}
                  disabled={ptyStatus !== "running"}
                  className="text-[11px] font-medium text-accent hover:underline disabled:cursor-not-allowed disabled:text-fg-3 disabled:no-underline"
                >
                  {selected ? "open" : "open pty"}
                </button>
              </div>
              <div className="text-[11px] text-fg-2">{subtitle}</div>
            </div>
          );
        })
      )}
    </aside>
  );
}
