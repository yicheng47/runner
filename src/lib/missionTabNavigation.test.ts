import { describe, expect, it } from "vitest";

import { missionTabInDirection } from "./missionTabNavigation";

describe("missionTabInDirection", () => {
  const sessions = ["lead", "coder", "reviewer"];

  it("enters the runner tabs from either side of the feed", () => {
    expect(missionTabInDirection(sessions, "feed", "next")).toBe("lead");
    expect(missionTabInDirection(sessions, "feed", "previous")).toBe(
      "reviewer",
    );
  });

  it("cycles runner tabs in order", () => {
    expect(missionTabInDirection(sessions, "lead", "next")).toBe("coder");
    expect(missionTabInDirection(sessions, "coder", "previous")).toBe("lead");
  });

  it("wraps through the feed at both ends", () => {
    expect(missionTabInDirection(sessions, "reviewer", "next")).toBe("feed");
    expect(missionTabInDirection(sessions, "lead", "previous")).toBe("feed");
  });

  it("does nothing when no runner tabs are open", () => {
    expect(missionTabInDirection([], "feed", "next")).toBeNull();
  });
});
