import { describe, expect, it } from "vitest";

import {
  directChatDisplayStatus,
  summarizeDirectChatGroupStatus,
} from "./directChatStatus";

describe("directChatDisplayStatus", () => {
  it("projects running sessions to their live activity state", () => {
    expect(directChatDisplayStatus("running", "idle")).toBe("idle");
    expect(directChatDisplayStatus("running", "busy")).toBe("busy");
  });

  it("keeps terminal lifecycle states as display states", () => {
    expect(directChatDisplayStatus("stopped", "busy")).toBe("stopped");
    expect(directChatDisplayStatus("crashed", "idle")).toBe("crashed");
  });
});

describe("summarizeDirectChatGroupStatus", () => {
  it("uses pane display state instead of raw running lifecycle state", () => {
    expect(summarizeDirectChatGroupStatus(["idle", "idle"], 2)).toMatchObject({
      status: "idle",
      count: 2,
      paneCount: 2,
      label: "2/2 idle",
    });
  });

  it("counts the highest-priority visible display state", () => {
    expect(
      summarizeDirectChatGroupStatus(["idle", "busy", "stopped"], 3),
    ).toMatchObject({
      status: "busy",
      count: 1,
      label: "1/3 busy",
    });
  });

  it("surfaces crashed panes before live activity", () => {
    expect(
      summarizeDirectChatGroupStatus(["busy", "crashed", "idle"], 3),
    ).toMatchObject({
      status: "crashed",
      count: 1,
      label: "1/3 crashed",
    });
  });
});
