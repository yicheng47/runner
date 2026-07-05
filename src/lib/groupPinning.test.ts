import { describe, expect, it } from "vitest";

import {
  groupPinTargets,
  pinnedSessionIds,
  shouldInheritPinOnAdd,
} from "./groupPinning";

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
