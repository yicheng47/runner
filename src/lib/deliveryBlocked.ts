import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import type { DeliveryBlockedEvent } from "./types";

export function useMissionDeliveryBlocked(
  missionId: string,
  sessionIds: string[],
): Record<string, DeliveryBlockedEvent> {
  const [blockedBySession, setBlockedBySession] = useState<
    Record<string, DeliveryBlockedEvent>
  >({});
  const sessionIdsKey = [...sessionIds].sort().join("\0");

  useEffect(() => {
    let cancelled = false;
    let unlistenBlocked: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    setBlockedBySession({});

    void Promise.all([
      listen<DeliveryBlockedEvent>(
        "router/delivery-blocked",
        ({ payload }) => {
          if (payload.mission_id !== missionId) return;
          setBlockedBySession((previous) => {
            if (!payload.blocked) {
              if (!(payload.session_id in previous)) return previous;
              const next = { ...previous };
              delete next[payload.session_id];
              return next;
            }
            return { ...previous, [payload.session_id]: payload };
          });
        },
      ),
      listen<{ session_id: string; mission_id: string | null }>(
        "session/exit",
        ({ payload }) => {
          if (payload.mission_id !== missionId) return;
          setBlockedBySession((previous) => {
            if (!(payload.session_id in previous)) return previous;
            const next = { ...previous };
            delete next[payload.session_id];
            return next;
          });
        },
      ),
    ]).then(([stopBlocked, stopExit]) => {
      if (cancelled) {
        stopBlocked();
        stopExit();
        return;
      }
      unlistenBlocked = stopBlocked;
      unlistenExit = stopExit;
    });

    return () => {
      cancelled = true;
      unlistenBlocked?.();
      unlistenExit?.();
    };
  }, [missionId]);

  useEffect(() => {
    const currentSessionIds = new Set(
      sessionIdsKey ? sessionIdsKey.split("\0") : [],
    );
    setBlockedBySession((previous) => {
      const next = Object.fromEntries(
        Object.entries(previous).filter(([sessionId]) =>
          currentSessionIds.has(sessionId),
        ),
      );
      return Object.keys(next).length === Object.keys(previous).length
        ? previous
        : next;
    });
  }, [sessionIdsKey]);

  return blockedBySession;
}
