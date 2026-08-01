import { Mail } from "lucide-react";

import { api } from "../lib/api";

export function InboxBlockedPill({
  sessionId,
  unreadCount,
  idle,
  narrow,
  onError,
}: {
  sessionId: string;
  unreadCount: number;
  idle: boolean;
  narrow: boolean;
  onError: (message: string) => void;
}) {
  const clearInput = () => {
    void api.session.injectStdin(sessionId, "\r").catch((error) => {
      onError(String(error));
    });
  };

  return (
    <div
      role="status"
      aria-label="Inbox delivery waiting"
      className="pointer-events-none absolute right-4 top-3 z-20 flex items-center gap-2 rounded-lg border border-warn/25 bg-panel px-3 py-2 text-xs shadow-[0_4px_16px_rgba(0,0,0,0.4)]"
    >
      <Mail aria-hidden className="h-3.5 w-3.5 shrink-0 text-warn" />
      <span className="whitespace-nowrap font-semibold text-warn">
        Inbox waiting{unreadCount > 1 ? ` (${unreadCount})` : ""}
      </span>
      {!narrow ? (
        <span className="whitespace-nowrap text-fg-2">
          — typing detected, delivery paused
        </span>
      ) : null}
      {idle ? (
        <button
          type="button"
          tabIndex={-1}
          onMouseDown={(event) => event.preventDefault()}
          onClick={clearInput}
          className="pointer-events-auto flex cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-warn/40 bg-warn/10 px-2 py-1 text-[11px] font-semibold leading-none text-warn transition-colors hover:bg-warn/15 focus:outline-none"
        >
          Clear input
          <span className="font-mono text-[10px] font-normal text-warn/70">
            ↵
          </span>
        </button>
      ) : null}
    </div>
  );
}
