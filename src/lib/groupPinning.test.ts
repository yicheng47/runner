import { describe, expect, it } from "vitest";

import {
  clusterActiveGroupRows,
  groupPinTargets,
  pinnedSessionIds,
  shouldInheritPinOnAdd,
} from "./groupPinning";

function rows(...ids: string[]): { session_id: string }[] {
  return ids.map((session_id) => ({ session_id }));
}

function order(list: readonly { session_id: string }[]): string[] {
  return list.map((r) => r.session_id);
}

describe("pinnedSessionIds", () => {
  it("collects the pinned rows' ids", () => {
    const ids = pinnedSessionIds([
      { session_id: "A", pinned: true },
      { session_id: "B", pinned: false },
      { session_id: "C", pinned: true },
    ]);
    expect([...ids].sort()).toEqual(["A", "C"]);
  });
});

describe("shouldInheritPinOnAdd", () => {
  it("pins the new chat when an existing member is pinned", () => {
    expect(shouldInheritPinOnAdd(["A", "B"], new Set(["B"]), "C")).toBe(true);
  });

  it("no-ops when no member is pinned (never unpins on add)", () => {
    expect(shouldInheritPinOnAdd(["A", "B"], new Set(), "C")).toBe(false);
    // Even an already-pinned chat joining an unpinned group keeps its pin
    // untouched — the decision is only ever "write pin", never "unpin".
    expect(shouldInheritPinOnAdd(["A"], new Set(["C"]), "C")).toBe(false);
  });

  it("skips the write when the new chat is already pinned", () => {
    expect(shouldInheritPinOnAdd(["A"], new Set(["A", "C"]), "C")).toBe(false);
  });

  it("ignores the new chat itself among the members", () => {
    // Defensive: caller passed the post-assign member list.
    expect(shouldInheritPinOnAdd(["A", "C"], new Set(), "C")).toBe(false);
  });

  it("no-ops with no existing members", () => {
    expect(shouldInheritPinOnAdd([], new Set(["X"]), "C")).toBe(false);
  });
});

describe("groupPinTargets", () => {
  it("fans a pin out to every unpinned member of the active group", () => {
    expect(groupPinTargets("A", ["A", "B", "C"], new Set(), true)).toEqual([
      "A",
      "B",
      "C",
    ]);
  });

  it("fans an unpin out to every pinned member", () => {
    expect(
      groupPinTargets("B", ["A", "B"], new Set(["A", "B"]), false),
    ).toEqual(["A", "B"]);
  });

  it("skips members already in the target state (drifted group converges)", () => {
    expect(
      groupPinTargets("B", ["A", "B", "C"], new Set(["A"]), true),
    ).toEqual(["B", "C"]);
  });

  it("targets only the toggled chat when it is not in the active group", () => {
    expect(groupPinTargets("X", ["A", "B"], new Set(), true)).toEqual(["X"]);
  });

  it("targets only the toggled chat when no group is active", () => {
    expect(groupPinTargets("X", [], new Set(["X"]), false)).toEqual(["X"]);
  });
});

describe("clusterActiveGroupRows", () => {
  it("renders members as one block in pane order at the highest-sorted member", () => {
    // Sidebar sort: P (pinned) first, then B, X, A. Group [A, B] anchors
    // at B's slot (the group's best-sorted member) in pane order A, B.
    const input = rows("P", "B", "X", "A");
    expect(order(clusterActiveGroupRows(input, ["A", "B"]))).toEqual([
      "P",
      "A",
      "B",
      "X",
    ]);
  });

  it("preserves non-members' relative order", () => {
    const input = rows("X", "A", "Y", "Z", "B");
    expect(order(clusterActiveGroupRows(input, ["B", "A"]))).toEqual([
      "X",
      "B",
      "A",
      "Y",
      "Z",
    ]);
  });

  it("keeps an unpinned group below the pinned cluster", () => {
    // P1/P2 pinned, group members A/B unpinned: the anchor is A (best
    // member), so the block must not climb above the pinned rows.
    const input = rows("P1", "P2", "A", "X", "B");
    expect(order(clusterActiveGroupRows(input, ["A", "B"]))).toEqual([
      "P1",
      "P2",
      "A",
      "B",
      "X",
    ]);
  });

  it("clusters inside the pinned section when the group is pinned", () => {
    // Group A/B pinned with an unrelated pinned chat P between them:
    // P stops interleaving but keeps its pinned-cluster spot.
    const input = rows("A", "P", "B", "X");
    expect(order(clusterActiveGroupRows(input, ["A", "B"]))).toEqual([
      "A",
      "B",
      "P",
      "X",
    ]);
  });

  it("returns the input unchanged when fewer than two members are listed", () => {
    const input = rows("A", "X", "B");
    expect(clusterActiveGroupRows(input, [])).toBe(input);
    expect(clusterActiveGroupRows(input, ["A"])).toBe(input);
    expect(clusterActiveGroupRows(input, ["A", "GONE"])).toBe(input);
  });

  it("ignores member ids missing from the rows (e.g. empty pane slots)", () => {
    const input = rows("X", "B", "A");
    expect(order(clusterActiveGroupRows(input, ["A", "GONE", "B"]))).toEqual([
      "X",
      "A",
      "B",
    ]);
  });
});
