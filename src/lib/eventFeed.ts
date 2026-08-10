import type { Event } from "./types";

const GROUP_WINDOW_MS = 5 * 60 * 1000;

export type FeedBlock =
  | { kind: "divider"; event: Event }
  | { kind: "message-group"; author: string; events: Event[] }
  | { kind: "signal"; event: Event }
  | { kind: "ask-card"; event: Event };

// Router watermarks/status stay in the NDJSON audit trail but add no value to
// the reading surface. ask_human is hidden because human_question projects the
// same prompt, choices, and attribution into the interactive card.
export function isHiddenSystemSignal(event: Event): boolean {
  return (
    event.kind === "signal" &&
    (event.type === "inbox_read" ||
      event.type === "runner_status" ||
      event.type === "ask_human")
  );
}

function isMessageLike(event: Event): boolean {
  return (
    event.kind === "message" ||
    (event.kind === "signal" &&
      (event.type === "human_said" ||
        event.type === "human_response" ||
        event.type === "mission_goal"))
  );
}

function isMissionGoal(event: Event): boolean {
  return event.kind === "signal" && event.type === "mission_goal";
}

function groupingRoute(event: Event): string {
  if (event.kind === "message") return `target:${event.to ?? ""}`;
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  if (event.type === "human_said") {
    return `target:${typeof payload.target === "string" ? payload.target : ""}`;
  }
  if (event.type === "human_response") {
    return `question:${
      typeof payload.question_id === "string" ? payload.question_id : ""
    }`;
  }
  return "";
}

function canJoinGroup(
  group: Extract<FeedBlock, { kind: "message-group" }>,
  event: Event,
) {
  if (group.author !== event.from) return false;
  const first = group.events[0];
  if (isMissionGoal(first) || isMissionGoal(event)) return false;
  if (groupingRoute(first) !== groupingRoute(event)) return false;
  const previous = Date.parse(group.events[group.events.length - 1].ts);
  const current = Date.parse(event.ts);
  const gap = current - previous;
  return Number.isFinite(gap) && gap >= 0 && gap <= GROUP_WINDOW_MS;
}

export function groupFeedBlocks(events: Event[]): FeedBlock[] {
  const blocks: FeedBlock[] = [];

  for (const event of events) {
    if (isHiddenSystemSignal(event)) continue;

    if (isMessageLike(event)) {
      const previous = blocks[blocks.length - 1];
      if (
        previous?.kind === "message-group" &&
        canJoinGroup(previous, event)
      ) {
        previous.events.push(event);
      } else {
        blocks.push({ kind: "message-group", author: event.from, events: [event] });
      }
      continue;
    }

    if (event.kind === "signal" && event.type === "mission_start") {
      blocks.push({ kind: "divider", event });
    } else if (event.kind === "signal" && event.type === "human_question") {
      blocks.push({ kind: "ask-card", event });
    } else {
      blocks.push({ kind: "signal", event });
    }
  }

  return blocks;
}

export function isHumanAuthored(event: Event): boolean {
  return (
    (event.kind === "message" && event.from === "human") ||
    (event.kind === "signal" &&
      (event.type === "human_said" || event.type === "human_response"))
  );
}
