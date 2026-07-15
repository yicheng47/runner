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
