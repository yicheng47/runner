// Right-hand rail in the mission workspace - one card per runner in the
// roster, showing PTY status (running / stopped / crashed), last
// `runner_status` (busy / idle), and the LEAD badge. Clicking the card
// (or its dedicated terminal-icon button) opens that slot's PTY as a
// new tab in the center pane and activates it.

import { Terminal } from "lucide-react";

import { CopyValueButton } from "./ui/CopyValueButton";
import type { SessionRow } from "../lib/api";
import { useT } from "../lib/i18n";

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
  const t = useT();
  return (
    <div className="flex flex-1 min-h-0 flex-col gap-3 overflow-y-auto px-5 pb-5">
      <div className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
        {t("Runner sessions")}
      </div>
      {sessions.length === 0 ? (
        <p className="text-xs text-fg-3">{t("No runner sessions yet.")}</p>
      ) : (
        sessions.map((s) => {
          const isLead = s.handle === leadHandle;
          const ptyStatus = s.status;
          const runnerStatus = status[s.handle];
          // Dot priority: crashed (red) > stopped (gray) > runner busy (green)
          // > runner idle (dim green). Default to "busy" when no runner_status
          // has landed yet — a freshly-started PTY is producing startup output.
          const dotClass =
            ptyStatus === "crashed"
              ? "bg-danger"
              : ptyStatus !== "running"
                ? "bg-fg-3"
                : runnerStatus === "idle"
                  ? "bg-accent/40"
                  : "bg-accent";
          const subtitle =
            ptyStatus === "running"
              ? runnerStatus
                ? runnerStatus
                : t("running")
              : ptyStatus;
          const selected = selectedSessionId === s.id;
          return (
            <div
              key={s.id}
              role="button"
              tabIndex={0}
              onClick={() => onOpenPty(s.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpenPty(s.id);
                }
              }}
              aria-pressed={selected}
              className={`group flex w-full cursor-pointer flex-col gap-1.5 rounded-md border bg-bg p-3 text-left transition-colors focus:outline-none focus-visible:border-accent focus-visible:ring-1 focus-visible:ring-accent/50 ${
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
                      {t("Lead")}
                    </span>
                  ) : null}
                </div>
                <button
                  type="button"
                  aria-label={t("Open @{handle} pty tab", { handle: s.handle })}
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenPty(s.id);
                  }}
                  className="inline-flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded border border-line text-fg-3 transition-colors hover:border-line-strong hover:text-fg"
                  title={t("Open PTY")}
                >
                  <Terminal aria-hidden className="h-3 w-3" />
                </button>
              </div>
              <div className="text-[11px] text-fg-2">{subtitle}</div>
              <div className="grid grid-cols-[72px_minmax(0,1fr)] gap-2 border-t border-line/70 pt-2 text-[10px] leading-snug">
                <span className="text-fg-3">session_key</span>
                <div className="flex min-w-0 items-start gap-1.5">
                  <span className="min-w-0 flex-1 break-all font-mono text-fg-2">
                    {s.agent_session_key ?? "NULL"}
                  </span>
                  <CopyValueButton
                    value={s.agent_session_key}
                    label={t("Copy @{handle} session_key", { handle: s.handle })}
                  />
                </div>
              </div>
            </div>
          );
        })
      )}
    </div>
  );
}
