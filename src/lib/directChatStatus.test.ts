import { describe, expect, it } from "vitest";

import { directChatDisplayStatus } from "./directChatStatus";

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
