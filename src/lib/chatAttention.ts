import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import { api, type DirectSessionEntry } from "./api";
import type {
  MissionActivityState,
  SessionActivityEvent,
  SessionActivityState,
} from "./types";

export type ChatAttentionState = "working" | "unread" | null;
export type SessionActivityMap = Record<
  string,
  SessionActivityState | undefined
>;

export function applySessionActivityEvents(
  snapshot: SessionActivityMap,
  events: SessionActivityEvent[],
): SessionActivityMap {
  const merged = { ...snapshot };
  for (const event of events) merged[event.session_id] = event.state;
  return merged;
}

export function tabHasUnreadCompletion(
  lastCompletedAt: string | null | undefined,
  lastViewedAt: string | null | undefined,
): boolean {
  if (!lastCompletedAt) return false;
  if (!lastViewedAt) return true;
  return Date.parse(lastCompletedAt) > Date.parse(lastViewedAt);
}

export function tabAttentionState(
  members: DirectSessionEntry[],
  activity: SessionActivityMap,
  lastCompletedAt: string | null | undefined,
  lastViewedAt: string | null | undefined,
): ChatAttentionState {
  if (
    members.some(
      (member) =>
        member.status === "running" &&
        activity[member.session_id] === "busy",
    )
  ) {
    return "working";
  }
  return tabHasUnreadCompletion(lastCompletedAt, lastViewedAt)
    ? "unread"
    : null;
}

export function rollupAttentionState(
  states: ChatAttentionState[],
): ChatAttentionState {
  if (states.includes("working")) return "working";
  if (states.includes("unread")) return "unread";
  return null;
}

export function missionAttentionState(
  anySessionLive: boolean,
  activity: MissionActivityState | null,
): ChatAttentionState {
  if (!anySessionLive || activity === "idle") return null;
  return "working";
}

export function useDirectSessionActivity(): SessionActivityMap {
  const [activity, setActivity] = useState<SessionActivityMap>({});

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let hydrating = true;
    const buffered: SessionActivityEvent[] = [];

    void (async () => {
      try {
        const stop = await listen<SessionActivityEvent>(
          "session/status",
          (event) => {
            if (hydrating) {
              buffered.push(event.payload);
              return;
            }
            setActivity((current) =>
              applySessionActivityEvents(current, [event.payload]),
            );
          },
        );
        if (cancelled) {
          stop();
          return;
        }
        unlisten = stop;

        let snapshot: SessionActivityMap = {};
        try {
          snapshot = await api.session.activitySnapshot();
        } catch {
          // The live stream remains authoritative if hydration fails.
        }
        if (cancelled) return;
        hydrating = false;
        setActivity(applySessionActivityEvents(snapshot, buffered));
      } catch {
        hydrating = false;
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return activity;
}
