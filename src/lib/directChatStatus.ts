import type { SessionActivityState, SessionStatus } from "./types";

export type DirectChatDisplayStatus =
  | SessionActivityState
  | "stopped"
  | "crashed";

export function directChatDisplayStatus(
  status: SessionStatus,
  activity: SessionActivityState | undefined,
): DirectChatDisplayStatus {
  if (status === "stopped" || status === "crashed") return status;
  return activity ?? "busy";
}

export interface DirectChatGroupStatusSummary {
  status: DirectChatDisplayStatus;
  count: number;
  paneCount: number;
  label: string;
}

const GROUP_STATUS_PRIORITY: readonly DirectChatDisplayStatus[] = [
  "crashed",
  "busy",
  "idle",
  "stopped",
];

export function summarizeDirectChatGroupStatus(
  statuses: readonly DirectChatDisplayStatus[],
  paneCount = statuses.length,
): DirectChatGroupStatusSummary {
  const counts: Record<DirectChatDisplayStatus, number> = {
    busy: 0,
    idle: 0,
    stopped: 0,
    crashed: 0,
  };
  for (const status of statuses) counts[status] += 1;

  const status =
    GROUP_STATUS_PRIORITY.find((candidate) => counts[candidate] > 0) ??
    "stopped";
  const count = counts[status];
  return {
    status,
    count,
    paneCount,
    label: `${count}/${paneCount} ${status}`,
  };
}
