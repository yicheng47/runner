import { describe, expect, it } from "vitest";

import { orderedRootNodeIdsAfterProjectDrop } from "./sidebarDnd";

describe("project drop ordering", () => {
  const nodes = [
    { id: "pinned-chat", parent_id: null, type: "tab" as const },
    { id: "project-a", parent_id: null, type: "project" as const },
    { id: "mission-a", parent_id: null, type: "mission" as const },
    { id: "project-b", parent_id: null, type: "project" as const },
    { id: "chat-b", parent_id: null, type: "tab" as const },
    { id: "project-c", parent_id: null, type: "project" as const },
    { id: "nested-chat", parent_id: "project-a", type: "tab" as const },
  ];

  it("reorders project slots while preserving non-project root order", () => {
    expect(orderedRootNodeIdsAfterProjectDrop(nodes, "project-b", 0)).toEqual([
      "pinned-chat",
      "project-b",
      "mission-a",
      "project-a",
      "chat-b",
      "project-c",
    ]);
  });

  it("moves a project to the final project slot without nested nodes", () => {
    expect(orderedRootNodeIdsAfterProjectDrop(nodes, "project-a", 2)).toEqual([
      "pinned-chat",
      "project-b",
      "mission-a",
      "project-c",
      "chat-b",
      "project-a",
    ]);
  });

  it("clamps an index beyond the project list", () => {
    expect(
      orderedRootNodeIdsAfterProjectDrop(
        nodes,
        "project-b",
        Number.MAX_SAFE_INTEGER,
      ),
    ).toEqual([
      "pinned-chat",
      "project-a",
      "mission-a",
      "project-c",
      "chat-b",
      "project-b",
    ]);
  });
});
