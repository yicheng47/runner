import { describe, expect, it } from "vitest";

import type { DirectSessionEntry } from "./api";
import {
  applySessionActivityEvents,
  missionAttentionState,
  rollupAttentionState,
  tabAttentionState,
} from "./chatAttention";

const member = (session_id: string, status: DirectSessionEntry["status"] = "running") =>
  ({ session_id, status }) as DirectSessionEntry;

describe("chat attention", () => {
  it("replays events received after a snapshot request over the snapshot", () => {
    expect(
      applySessionActivityEvents(
        { A: "busy", B: "idle" },
        [
          { session_id: "A", state: "idle", source: "forwarder" },
          { session_id: "B", state: "busy", source: "resume" },
        ],
      ),
    ).toEqual({ A: "idle", B: "busy" });
  });

  it("aggregates a multi-pane tab until its final busy member settles", () => {
    const members = [member("A"), member("B")];
    expect(tabAttentionState(members, { A: "idle", B: "busy" }, null, null)).toBe(
      "working",
    );
    expect(tabAttentionState(members, { A: "idle", B: "idle" }, null, null)).toBeNull();
  });

  it("does not synthesize unread state from initial idle hydration", () => {
    expect(tabAttentionState([member("A")], { A: "idle" }, null, null)).toBeNull();
  });

  it("keeps unread under new work and restores it when work settles", () => {
    const members = [member("A")];
    expect(
      tabAttentionState(
        members,
        { A: "busy" },
        "2026-07-13T10:00:00Z",
        "2026-07-13T09:00:00Z",
      ),
    ).toBe("working");
    expect(
      tabAttentionState(
        members,
        { A: "idle" },
        "2026-07-13T10:00:00Z",
        "2026-07-13T09:00:00Z",
      ),
    ).toBe("unread");
  });

  it("ignores busy activity for stopped members", () => {
    expect(
      tabAttentionState([member("A", "stopped")], { A: "busy" }, null, null),
    ).toBeNull();
  });

  it("rolls collapsed children up with working precedence", () => {
    expect(rollupAttentionState([null, "unread"])).toBe("unread");
    expect(rollupAttentionState(["unread", "working"])).toBe("working");
    expect(rollupAttentionState([null, null])).toBeNull();
  });

  it("maps mission activity to the chat working indicator", () => {
    expect(missionAttentionState(true, "busy")).toBe("working");
    expect(missionAttentionState(true, null)).toBe("working");
    expect(missionAttentionState(true, "idle")).toBeNull();
    expect(missionAttentionState(false, "busy")).toBeNull();
  });
});
