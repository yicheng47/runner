import { describe, expect, it } from "vitest";

import {
  applyPresetPure,
  assignSessionPure,
  closePanePure,
  deserializeLayout,
  isGroupActiveFor,
  leaves,
  removeSessionPure,
  serializeLayout,
  setSizesPure,
  visibleSessionIds,
  type PaneLayout,
  type PaneSplit,
} from "./paneLayout";

function slotSessions(layout: PaneLayout): (string | null)[] {
  return leaves(layout.root).map((l) => l.sessionId);
}

describe("applyPresetPure", () => {
  it("builds every preset with the right pane count and shape", () => {
    expect(leaves(applyPresetPure("single", null, []).root)).toHaveLength(1);
    expect(leaves(applyPresetPure("cols-2", null, []).root)).toHaveLength(2);
    expect(leaves(applyPresetPure("rows-2", null, []).root)).toHaveLength(2);
    expect(leaves(applyPresetPure("main-2", null, []).root)).toHaveLength(3);
    expect(leaves(applyPresetPure("cols-3", null, []).root)).toHaveLength(3);
    expect(leaves(applyPresetPure("rows-3", null, []).root)).toHaveLength(3);

    const cols = applyPresetPure("cols-2", null, []).root as PaneSplit;
    expect(cols.orientation).toBe("row");
    const rows = applyPresetPure("rows-2", null, []).root as PaneSplit;
    expect(rows.orientation).toBe("col");

    // 1-big+2-stacked: row of [big leaf, col split].
    const main = applyPresetPure("main-2", null, []).root as PaneSplit;
    expect(main.orientation).toBe("row");
    expect(main.a.kind).toBe("leaf");
    expect(main.b.kind).toBe("split");
    expect((main.b as PaneSplit).orientation).toBe("col");
  });

  it("gives the focused chat the biggest slot and fills the rest in order", () => {
    const layout = applyPresetPure("main-2", "B", ["A", "B", "C"]);
    expect(slotSessions(layout)).toEqual(["B", "A", "C"]);
  });

  it("leaves extra slots empty and focuses the first empty pane", () => {
    const layout = applyPresetPure("cols-3", "A", ["A"]);
    expect(slotSessions(layout)).toEqual(["A", null, null]);
    const firstEmpty = leaves(layout.root).find((l) => l.sessionId === null);
    expect(layout.focusedPaneId).toBe(firstEmpty!.id);
  });

  it("focuses the focused chat's pane when every slot is filled", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const focused = leaves(layout.root).find((l) => l.sessionId === "A");
    expect(layout.focusedPaneId).toBe(focused!.id);
  });

  it("drops overflow chats when the preset has fewer slots", () => {
    const layout = applyPresetPure("single", "B", ["A", "B", "C"]);
    expect(slotSessions(layout)).toEqual(["B"]);
  });
});

describe("assignSessionPure", () => {
  it("assigns a session to the target pane", () => {
    const layout = applyPresetPure("cols-2", "A", ["A"]);
    const target = leaves(layout.root)[1];
    const next = assignSessionPure(layout, target.id, "B");
    expect(slotSessions(next)).toEqual(["A", "B"]);
  });

  it("move-not-copy: clears the old slot when the session is visible elsewhere", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const [first] = leaves(layout.root);
    const next = assignSessionPure(layout, first.id, "B");
    expect(slotSessions(next)).toEqual(["B", null]);
    expect(visibleSessionIds(next.root)).toEqual(["B"]);
  });

  it("replaces the target pane's previous session (it drops to the hidden stack)", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const [first] = leaves(layout.root);
    const next = assignSessionPure(layout, first.id, "C");
    expect(slotSessions(next)).toEqual(["C", "B"]);
  });
});

describe("closePanePure", () => {
  it("collapses the focused pane and promotes the sibling", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const next = closePanePure(layout, layout.focusedPaneId);
    expect(next.preset).toBe("single");
    expect(slotSessions(next)).toEqual(["B"]);
    expect(next.focusedPaneId).toBe(leaves(next.root)[0].id);
  });

  it("collapsing the big pane of main-2 leaves the stacked pair", () => {
    const layout = applyPresetPure("main-2", "A", ["A", "B", "C"]);
    const big = leaves(layout.root)[0];
    const next = closePanePure(layout, big.id);
    expect(next.preset).toBe("rows-2");
    expect(slotSessions(next)).toEqual(["B", "C"]);
  });

  it("collapsing a stacked pane of main-2 leaves two columns", () => {
    const layout = applyPresetPure("main-2", "A", ["A", "B", "C"]);
    const last = leaves(layout.root)[2];
    const next = closePanePure(layout, last.id);
    expect(next.preset).toBe("cols-2");
    expect(slotSessions(next)).toEqual(["A", "B"]);
    // A non-focused pane closed → focus stays put.
    expect(next.focusedPaneId).toBe(layout.focusedPaneId);
  });

  it("is a no-op on a single pane", () => {
    const layout = applyPresetPure("single", "A", ["A"]);
    expect(closePanePure(layout, layout.focusedPaneId)).toBe(layout);
  });
});

describe("removeSessionPure", () => {
  it("empties the pane that showed the session", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const next = removeSessionPure(layout, "B");
    expect(slotSessions(next)).toEqual(["A", null]);
  });

  it("returns the same layout when the session isn't visible", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    expect(removeSessionPure(layout, "C")).toBe(layout);
  });
});

describe("setSizesPure", () => {
  it("updates the matching split's sizes only", () => {
    const layout = applyPresetPure("main-2", "A", ["A", "B", "C"]);
    const outer = layout.root as PaneSplit;
    const next = setSizesPure(layout, outer.id, [70, 30]);
    expect((next.root as PaneSplit).sizes).toEqual([70, 30]);
    expect(((next.root as PaneSplit).b as PaneSplit).sizes).toEqual([50, 50]);
  });
});

describe("isGroupActiveFor", () => {
  it("is true only for member chats of a split layout", () => {
    const split = applyPresetPure("cols-2", "A", ["A", "B"]);
    expect(isGroupActiveFor(split, "A")).toBe(true);
    expect(isGroupActiveFor(split, "B")).toBe(true);
    expect(isGroupActiveFor(split, "C")).toBe(false);
    expect(isGroupActiveFor(split, null)).toBe(false);
    const single = applyPresetPure("single", "A", ["A"]);
    expect(isGroupActiveFor(single, "A")).toBe(false);
  });
});

describe("serializeLayout / deserializeLayout", () => {
  it("round-trips preset, slots, sizes, and focus", () => {
    const layout = setSizesPure(
      applyPresetPure("main-2", "B", ["A", "B"]),
      (applyPresetPure("main-2", "B", ["A", "B"]).root as PaneSplit).id,
      [70, 30],
    );
    const restored = deserializeLayout(serializeLayout(layout));
    expect(restored).not.toBeNull();
    expect(restored!.preset).toBe("main-2");
    expect(slotSessions(restored!)).toEqual(["B", "A", null]);
    expect((restored!.root as PaneSplit).sizes).toEqual([70, 30]);
    const focusedBefore = leaves(layout.root).findIndex(
      (l) => l.id === layout.focusedPaneId,
    );
    const focusedAfter = leaves(restored!.root).findIndex(
      (l) => l.id === restored!.focusedPaneId,
    );
    expect(focusedAfter).toBe(focusedBefore);
  });

  it("round-trips the group name, and drops it for single layouts", () => {
    const named = applyPresetPure("cols-2", "A", ["A", "B"], "review pair");
    expect(named.name).toBe("review pair");
    const restored = deserializeLayout(serializeLayout(named));
    expect(restored!.name).toBe("review pair");
    expect(applyPresetPure("single", "A", ["A"], "review pair").name).toBeNull();
  });

  it("rejects malformed payloads", () => {
    expect(deserializeLayout("not json")).toBeNull();
    expect(deserializeLayout("null")).toBeNull();
    expect(deserializeLayout(JSON.stringify({ preset: "nope" }))).toBeNull();
  });

  it("survives garbage sizes and out-of-range focus", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);
    const parsed = JSON.parse(serializeLayout(layout)) as Record<
      string,
      unknown
    >;
    parsed.sizes = { bogus: [0, 200] };
    parsed.focusedSlot = 99;
    const restored = deserializeLayout(JSON.stringify(parsed));
    expect(restored).not.toBeNull();
    expect(slotSessions(restored!)).toEqual(["A", "B"]);
    expect((restored!.root as PaneSplit).sizes).toEqual([50, 50]);
    expect(restored!.focusedPaneId).toBe(leaves(restored!.root)[0].id);
  });
});
