import { describe, expect, it } from "vitest";

import {
  completeUnpinnedScopeOrder,
  orderedPinnedNodeIdsAfterDrop,
  orderedRootNodeIdsAfterProjectDrop,
  orderedVisibleNodeIdsAfterDrop,
} from "./sidebarDnd";

describe("project drop ordering", () => {
  const nodes = [
    {
      id: "pinned-chat",
      parent_id: null,
      type: "tab" as const,
      pinned_position: 0,
    },
    {
      id: "project-a",
      parent_id: null,
      type: "project" as const,
      pinned_position: null,
    },
    {
      id: "mission-a",
      parent_id: null,
      type: "mission" as const,
      pinned_position: null,
    },
    {
      id: "project-b",
      parent_id: null,
      type: "project" as const,
      pinned_position: null,
    },
    {
      id: "chat-b",
      parent_id: null,
      type: "tab" as const,
      pinned_position: null,
    },
    {
      id: "project-c",
      parent_id: null,
      type: "project" as const,
      pinned_position: null,
    },
    {
      id: "nested-chat",
      parent_id: "project-a",
      type: "tab" as const,
      pinned_position: null,
    },
  ];

  it("reorders project slots while excluding a pinned root tab", () => {
    expect(orderedRootNodeIdsAfterProjectDrop(nodes, "project-b", 0)).toEqual([
      "project-b",
      "mission-a",
      "project-a",
      "chat-b",
      "project-c",
    ]);
  });

  it("moves a project to the final project slot without nested nodes", () => {
    expect(orderedRootNodeIdsAfterProjectDrop(nodes, "project-a", 2)).toEqual([
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
      "project-a",
      "mission-a",
      "project-c",
      "chat-b",
      "project-b",
    ]);
  });
});

describe("sidebar ordered-id construction", () => {
  it("builds the complete pinned order while preserving hidden pinned slots", () => {
    const nodes = [
      {
        id: "root-pinned",
        parent_id: null,
        type: "tab" as const,
        pinned_position: 0,
      },
      {
        id: "hidden-pinned",
        parent_id: null,
        type: "mission" as const,
        pinned_position: 1,
      },
      {
        id: "nested-pinned",
        parent_id: "project-a",
        type: "tab" as const,
        pinned_position: 2,
      },
      {
        id: "ordinary",
        parent_id: null,
        type: "tab" as const,
        pinned_position: null,
      },
    ];

    expect(
      orderedPinnedNodeIdsAfterDrop(
        nodes,
        ["root-pinned", "nested-pinned"],
        "nested-pinned",
        0,
      ),
    ).toEqual(["nested-pinned", "hidden-pinned", "root-pinned"]);
    expect(
      orderedPinnedNodeIdsAfterDrop(
        nodes,
        ["root-pinned", "nested-pinned"],
        "hidden-pinned",
        0,
      ),
    ).toEqual(["root-pinned", "hidden-pinned", "nested-pinned"]);
  });

  it("excludes pinned members from complete root scope payloads", () => {
    const nodes = [
      {
        id: "pinned-chat",
        parent_id: null,
        type: "tab" as const,
        pinned_position: 0,
      },
      {
        id: "project",
        parent_id: null,
        type: "project" as const,
        pinned_position: null,
      },
      {
        id: "visible-chat",
        parent_id: null,
        type: "tab" as const,
        pinned_position: null,
      },
      {
        id: "hidden-mission",
        parent_id: null,
        type: "mission" as const,
        pinned_position: null,
      },
    ];

    expect(
      completeUnpinnedScopeOrder(
        nodes,
        null,
        "visible-chat",
        ["visible-chat"],
      ),
    ).toEqual(["project", "visible-chat", "hidden-mission"]);
  });

  it("reorders visible unpinned rows without pin-tier constraints", () => {
    expect(
      orderedVisibleNodeIdsAfterDrop(["a", "b", "c"], "c", 0),
    ).toEqual(["c", "a", "b"]);
  });
});
