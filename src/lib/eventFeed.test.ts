import { describe, expect, it } from "vitest";

import type { Event } from "./types";
import { groupFeedBlocks, isHumanAuthored } from "./eventFeed";

let nextId = 0;

function event(overrides: Partial<Event> = {}): Event {
  return {
    id: `01KTEST${nextId++}`,
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

describe("groupFeedBlocks", () => {
  it("groups consecutive messages from the same author inside five minutes", () => {
    const first = event({ ts: "2026-08-01T00:00:00Z" });
    const second = event({ ts: "2026-08-01T00:05:00Z" });

    expect(groupFeedBlocks([first, second])).toEqual([
      { kind: "message-group", author: "coder", events: [first, second] },
    ]);
  });

  it("starts a new group outside the five-minute window", () => {
    const first = event({ ts: "2026-08-01T00:00:00Z" });
    const second = event({ ts: "2026-08-01T00:05:01Z" });

    expect(groupFeedBlocks([first, second])).toEqual([
      { kind: "message-group", author: "coder", events: [first] },
      { kind: "message-group", author: "coder", events: [second] },
    ]);
  });

  it("breaks a message group around an interleaved signal", () => {
    const first = event({ ts: "2026-08-01T00:00:00Z" });
    const signal = event({
      kind: "signal",
      type: "ask_lead",
      ts: "2026-08-01T00:01:00Z",
    });
    const second = event({ ts: "2026-08-01T00:02:00Z" });

    expect(groupFeedBlocks([first, signal, second])).toEqual([
      { kind: "message-group", author: "coder", events: [first] },
      { kind: "signal", event: signal },
      { kind: "message-group", author: "coder", events: [second] },
    ]);
  });

  it("classifies mission_goal and human_said as separate message-like blocks", () => {
    const goal = event({
      kind: "signal",
      from: "human",
      type: "mission_goal",
      payload: { text: "Ship it" },
    });
    const said = event({
      kind: "signal",
      from: "human",
      type: "human_said",
      ts: "2026-08-01T00:01:00Z",
      payload: { text: "Keep going" },
    });

    expect(groupFeedBlocks([goal, said])).toEqual([
      { kind: "message-group", author: "human", events: [goal] },
      { kind: "message-group", author: "human", events: [said] },
    ]);
  });

  it("starts a new group when the routing target changes", () => {
    const direct = event({ to: "reviewer" });
    const broadcast = event({ ts: "2026-08-01T00:01:00Z" });
    const otherDirect = event({
      to: "lead",
      ts: "2026-08-01T00:02:00Z",
    });

    expect(groupFeedBlocks([direct, broadcast, otherDirect])).toEqual([
      { kind: "message-group", author: "coder", events: [direct] },
      { kind: "message-group", author: "coder", events: [broadcast] },
      { kind: "message-group", author: "coder", events: [otherDirect] },
    ]);
  });

  it("keeps human signals for different routes in separate groups", () => {
    const said = event({
      kind: "signal",
      from: "human",
      type: "human_said",
      payload: { text: "First", target: "reviewer" },
    });
    const otherSaid = event({
      kind: "signal",
      from: "human",
      type: "human_said",
      ts: "2026-08-01T00:01:00Z",
      payload: { text: "Second", target: "lead" },
    });
    const response = event({
      kind: "signal",
      from: "human",
      type: "human_response",
      ts: "2026-08-01T00:02:00Z",
      payload: { choice: "yes", question_id: "question-1" },
    });
    const otherResponse = event({
      kind: "signal",
      from: "human",
      type: "human_response",
      ts: "2026-08-01T00:03:00Z",
      payload: { choice: "no", question_id: "question-2" },
    });

    expect(
      groupFeedBlocks([said, otherSaid, response, otherResponse]),
    ).toEqual([
      { kind: "message-group", author: "human", events: [said] },
      { kind: "message-group", author: "human", events: [otherSaid] },
      { kind: "message-group", author: "human", events: [response] },
      { kind: "message-group", author: "human", events: [otherResponse] },
    ]);
  });

  it("hides raw ask_human signals", () => {
    const askHuman = event({ kind: "signal", type: "ask_human" });

    expect(groupFeedBlocks([askHuman])).toEqual([]);
  });

  it("maps mission_start to a divider", () => {
    const missionStart = event({ kind: "signal", type: "mission_start" });

    expect(groupFeedBlocks([missionStart])).toEqual([
      { kind: "divider", event: missionStart },
    ]);
  });

  it("maps human_question to an ask card", () => {
    const question = event({ kind: "signal", type: "human_question" });

    expect(groupFeedBlocks([question])).toEqual([
      { kind: "ask-card", event: question },
    ]);
  });
});
