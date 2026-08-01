import { describe, expect, it } from "vitest";

import type { Event } from "./types";
import { isHumanAuthored } from "./eventFeed";

function event(overrides: Partial<Event>): Event {
  return {
    id: "01KTEST",
    ts: "2026-08-01T00:00:00Z",
    crew_id: "crew-1",
    mission_id: "mission-1",
    kind: "message",
    from: "coder",
    to: null,
    payload: { text: "hello" },
    ...overrides,
  };
}

describe("isHumanAuthored", () => {
  it("classifies human message events as authored by the operator", () => {
    expect(isHumanAuthored(event({ from: "human" }))).toBe(true);
    expect(isHumanAuthored(event({ from: "coder" }))).toBe(false);
  });

  it("preserves human signal classification", () => {
    expect(
      isHumanAuthored(
        event({ kind: "signal", from: "human", type: "human_said" }),
      ),
    ).toBe(true);
    expect(
      isHumanAuthored(
        event({ kind: "signal", from: "human", type: "human_response" }),
      ),
    ).toBe(true);
    expect(
      isHumanAuthored(
        event({ kind: "signal", from: "human", type: "mission_goal" }),
      ),
    ).toBe(false);
  });
});
