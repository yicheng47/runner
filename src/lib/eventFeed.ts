import type { Event } from "./types";

export function isHumanAuthored(event: Event): boolean {
  return (
    (event.kind === "message" && event.from === "human") ||
    (event.kind === "signal" &&
      (event.type === "human_said" || event.type === "human_response"))
  );
}
