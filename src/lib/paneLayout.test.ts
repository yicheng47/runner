import { afterEach, describe, expect, it, vi } from "vitest";

import {
  activatePaneLayoutForSession,
  applyPreset,
  applyPresetPure,
  assignSessionToPane,
  assignSessionPure,
  closePanePure,
  deserializeLayout,
  focusPane,
  getPaneLayout,
  getPaneLayouts,
  getPaneLayoutsForTest,
  hydrateLayoutSet,
  isGroupActiveFor,
  isFreshlyAssigned,
  leaves,
  newChatTargetPane,
  removeSessionPure,
  resetPaneLayoutsForTest,
  serializeLayout,
  serializeLayoutSet,
  setGroupNameForSession,
  setRouteAnchorSession,
  setSizesPure,
  subscribePaneLayout,
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

describe("newChatTargetPane", () => {
  it("returns the active group's empty pane even when it is not focused", () => {
    const layout = removeSessionPure(
      applyPresetPure("cols-2", "A", ["A", "B"]),
      "B",
    );

    expect(slotSessions(layout)).toEqual(["A", null]);
    expect(layout.focusedPaneId).toBe(leaves(layout.root)[0].id);
    expect(newChatTargetPane(layout, "A")).toBe(leaves(layout.root)[1].id);
  });

  it("returns the emptied pane after archiving the URL-owner chat", () => {
    const emptied = removeSessionPure(
      applyPresetPure("cols-2", "A", ["A", "B"]),
      "A",
    );
    const all = leaves(emptied.root);
    const survivorFocused = { ...emptied, focusedPaneId: all[1].id };

    expect(slotSessions(survivorFocused)).toEqual([null, "B"]);
    expect(newChatTargetPane(survivorFocused, "B")).toBe(all[0].id);
  });

  it("returns null when the active group has no empty pane", () => {
    const layout = applyPresetPure("cols-2", "A", ["A", "B"]);

    expect(newChatTargetPane(layout, "A")).toBeNull();
  });

  it("returns null for an inactive group or single layout", () => {
    const split = applyPresetPure("cols-2", "A", ["A", "B"]);
    const single = applyPresetPure("single", "A", ["A"]);

    expect(newChatTargetPane(split, "C")).toBeNull();
    expect(newChatTargetPane(single, "A")).toBeNull();
  });

  it("returns the first empty pane in pane order", () => {
    const layout = applyPresetPure("main-2", "A", ["A"]);
    const all = leaves(layout.root);
    const unfocusedSecondEmpty = { ...layout, focusedPaneId: all[2].id };

    expect(slotSessions(unfocusedSecondEmpty)).toEqual(["A", null, null]);
    expect(newChatTargetPane(unfocusedSecondEmpty, "A")).toBe(all[1].id);
  });
});

describe("pane layout tabs", () => {
  afterEach(() => {
    resetPaneLayoutsForTest();
  });

  it("keeps an existing pane tab when creating panes from a non-member chat", () => {
    applyPreset("cols-2", "A", ["A"]);
    applyPreset("cols-2", "C", ["C"]);

    expect(getPaneLayoutsForTest()).toHaveLength(2);
    expect(slotSessions(getPaneLayout("A"))).toEqual(["A", null]);
    expect(slotSessions(getPaneLayout("C"))).toEqual(["C", null]);

    activatePaneLayoutForSession("A");

    expect(slotSessions(getPaneLayout())).toEqual(["A", null]);
  });

  it("moves a session out of its old pane tab when assigned to another one", () => {
    applyPreset("cols-2", "A", ["A"]);
    applyPreset("cols-2", "C", ["C"]);

    activatePaneLayoutForSession("A");
    assignSessionToPane("p2", "C");

    expect(getPaneLayoutsForTest()).toHaveLength(1);
    expect(slotSessions(getPaneLayout("C"))).toEqual(["A", "C"]);
  });

  it("reactivates an inactive pane tab and focuses the clicked member", () => {
    applyPreset("cols-2", "A", ["A", "B"]);
    const bLeaf = leaves(getPaneLayout("B").root).find(
      (leaf) => leaf.sessionId === "B",
    )!;
    focusPane(bLeaf.id);
    applyPreset("cols-2", "C", ["C"]);

    const reactivated = activatePaneLayoutForSession("A");
    const aLeaf = leaves(reactivated.root).find(
      (leaf) => leaf.sessionId === "A",
    )!;
    focusPane(aLeaf.id);

    expect(getPaneLayout("A").focusedPaneId).toBe(aLeaf.id);
  });
});

describe("setGroupNameForSession", () => {
  afterEach(() => {
    resetPaneLayoutsForTest();
  });

  it("renames a background group without activating it", () => {
    applyPreset("cols-2", "A", ["A", "B"]);
    applyPreset("cols-2", "C", ["C", "D"]);
    activatePaneLayoutForSession("A");

    setGroupNameForSession("C", "review pair");

    expect(getPaneLayout()).toBe(getPaneLayouts()[0]);
    expect(getPaneLayouts()[1].name).toBe("review pair");
  });

  it("clears a group name with blank input", () => {
    applyPreset("cols-2", "A", ["A", "B"], "review pair");

    setGroupNameForSession("A", "   ");

    expect(getPaneLayout("A").name).toBeNull();
  });
});

describe("hydrateLayoutSet (cross-window sync)", () => {
  afterEach(() => {
    vi.useRealTimers();
    resetPaneLayoutsForTest();
  });

  it("converges tab membership from another window and notifies subscribers", () => {
    // Local state: one tab {A,B}, focused on A (this window's active tab).
    applyPreset("cols-2", "A", ["A", "B"]);

    // A remote window's serialized set: our tab plus a second one it made
    // active.
    const remote = serializeLayoutSet(
      [
        applyPresetPure("cols-2", "A", ["A", "B"]),
        applyPresetPure("cols-2", "C", ["C", "D"]),
      ],
      1,
    );

    let notified = 0;
    const unsub = subscribePaneLayout(() => {
      notified += 1;
    });
    const applied = hydrateLayoutSet(remote);
    unsub();

    expect(applied).toBe(true);
    expect(notified).toBe(1);
    const tabs = getPaneLayouts();
    expect(tabs).toHaveLength(2);
    expect(slotSessions(tabs[0])).toEqual(["A", "B"]);
    expect(slotSessions(tabs[1])).toEqual(["C", "D"]);
  });

  it("keeps this window on its route tab instead of adopting the sender's active tab", () => {
    // This window's route owns A (its tab is {A,B}).
    applyPreset("cols-2", "A", ["A", "B"]);
    setRouteAnchorSession("A");

    // Sender's set puts A's tab at index 1 and makes {C,D} (index 0) active,
    // so neither the sender's activeIndex nor a positional clamp lands on it.
    const remote = serializeLayoutSet(
      [
        applyPresetPure("cols-2", "C", ["C", "D"]),
        applyPresetPure("cols-2", "A", ["A", "B"]),
      ],
      0,
    );

    hydrateLayoutSet(remote);

    // Active tab tracks the route session A, so a local closePane/focusPane
    // can't hit the wrong tab.
    expect(slotSessions(getPaneLayout())).toEqual(["A", "B"]);
    expect(getPaneLayout()).toBe(getPaneLayouts()[1]);
  });

  it("anchors on the route session, not a remotely-changed focused pane", () => {
    // Route owns A; its tab is {A,B}, focused on A.
    applyPreset("cols-2", "A", ["A", "B"]);
    setRouteAnchorSession("A");

    // A remote change moves the tab's focused pane to B (focus is synced).
    const tabAB = applyPresetPure("cols-2", "A", ["A", "B"]);
    const bLeaf = leaves(tabAB.root).find((l) => l.sessionId === "B")!;
    hydrateLayoutSet(
      serializeLayoutSet([{ ...tabAB, focusedPaneId: bLeaf.id }], 0),
    );

    // Then a second remote change splits B into its own tab while A stays put.
    hydrateLayoutSet(
      serializeLayoutSet(
        [
          applyPresetPure("single", "A", ["A"]),
          applyPresetPure("cols-2", "B", ["B", "X"]),
        ],
        1,
      ),
    );

    // The window follows A (its route), not the remotely-moved focus on B.
    expect(getPaneLayout()).toBe(getPaneLayouts()[0]);
    expect(visibleSessionIds(getPaneLayout().root)).toEqual(["A"]);
  });

  it("follows the anchored session when a remote change moves its tab", () => {
    // Two tabs; this window's route owns A and is looking at A's tab.
    applyPreset("cols-2", "A", ["A"]);
    applyPreset("cols-2", "B", ["B"]);
    activatePaneLayoutForSession("A");
    setRouteAnchorSession("A");
    expect(visibleSessionIds(getPaneLayout().root)).toEqual(["A"]);

    // Remote regroups A and B into a single pair; the separate tabs are gone.
    const remote = serializeLayoutSet(
      [applyPresetPure("cols-2", "B", ["B", "A"])],
      0,
    );
    hydrateLayoutSet(remote);

    // Active tab tracks A into the merged pair rather than falling out of it.
    expect(getPaneLayouts()).toHaveLength(1);
    expect(visibleSessionIds(getPaneLayout().root)).toContain("A");
  });

  it("does not let a background-tab activation rewrite the route anchor", () => {
    // Route owns A; a separate background tab holds C/D.
    applyPreset("cols-2", "A", ["A", "B"]);
    applyPreset("cols-2", "C", ["C", "D"]);
    activatePaneLayoutForSession("A"); // view A's tab
    setRouteAnchorSession("A"); // route owns A

    // Activating the background C/D tab (as setGroupName does when naming it)
    // must not navigate — the route anchor must stay A.
    activatePaneLayoutForSession("C");

    // A remote change moves C elsewhere; A stays in its own tab (index 0).
    const remote = serializeLayoutSet(
      [
        applyPresetPure("cols-2", "A", ["A", "B"]),
        applyPresetPure("cols-2", "C", ["C", "X"]),
      ],
      1,
    );
    hydrateLayoutSet(remote);

    // Anchored on A, not the just-activated C, so we stay on A's tab.
    expect(getPaneLayout()).toBe(getPaneLayouts()[0]);
    expect(slotSessions(getPaneLayout())).toEqual(["A", "B"]);
  });

  it("marks hydrated members fresh so the vanished-session sweep spares them", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    applyPreset("single", "A", ["A"]);

    // Remote fills a pane with a chat this window's row cache hasn't seen.
    const remote = serializeLayoutSet(
      [applyPresetPure("cols-2", "A", ["A", "just-made-elsewhere"])],
      0,
    );
    hydrateLayoutSet(remote);

    expect(isFreshlyAssigned("just-made-elsewhere")).toBe(true);
  });

  it("no-ops and returns false on a malformed payload", () => {
    applyPreset("cols-2", "A", ["A", "B"]);
    const before = getPaneLayouts();

    let notified = 0;
    const unsub = subscribePaneLayout(() => {
      notified += 1;
    });
    expect(hydrateLayoutSet("not json")).toBe(false);
    expect(hydrateLayoutSet("null")).toBe(false);
    unsub();

    expect(notified).toBe(0);
    expect(getPaneLayouts()).toBe(before);
  });
});

describe("fresh assignments", () => {
  afterEach(() => {
    vi.useRealTimers();
    resetPaneLayoutsForTest();
  });

  it("marks a session fresh immediately after assignment", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);

    assignSessionToPane("p1", "fresh-session");

    expect(isFreshlyAssigned("fresh-session")).toBe(true);
  });

  it("does not mark never-assigned sessions fresh", () => {
    expect(isFreshlyAssigned("never-assigned-session")).toBe(false);
  });

  it("expires freshness after the assignment TTL", () => {
    vi.useFakeTimers();
    vi.setSystemTime(2_000);

    assignSessionToPane("p1", "expired-session");
    vi.setSystemTime(17_000);

    expect(isFreshlyAssigned("expired-session")).toBe(false);
  });
});

describe("archive reconciliation", () => {
  afterEach(() => {
    resetPaneLayoutsForTest();
  });

  it("preserves the active tab by stable id when an earlier tab disappears", async () => {
    const { removeArchivedSessionFromLayout } = await import("./paneLayout");
    resetPaneLayoutsForTest(
      [
        { ...applyPresetPure("single", "A", ["A"]), id: "tab-a" },
        { ...applyPresetPure("single", "B", ["B"]), id: "tab-b" },
        { ...applyPresetPure("single", "C", ["C"]), id: "tab-c" },
      ],
      1,
    );

    removeArchivedSessionFromLayout("A");

    expect(getPaneLayouts().map((layout) => layout.id)).toEqual([
      "tab-b",
      "tab-c",
    ]);
    expect(getPaneLayout().id).toBe("tab-b");
  });
});

describe("serializeLayout / deserializeLayout", () => {
  it("round-trips preset, slots, and sizes while focus stays view state", () => {
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
    const focusedAfter = leaves(restored!.root).findIndex(
      (l) => l.id === restored!.focusedPaneId,
    );
    expect(focusedAfter).toBe(0);
  });

  it("keeps the tab name out of the layout blob but reads legacy names", () => {
    const named = applyPresetPure("cols-2", "A", ["A", "B"], "review pair");
    expect(named.name).toBe("review pair");
    expect(deserializeLayout(serializeLayout(named))!.name).toBeNull();
    const legacy = JSON.parse(serializeLayout(named));
    legacy.name = "review pair";
    expect(deserializeLayout(JSON.stringify(legacy))!.name).toBe("review pair");
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

describe("cold-start hydration", () => {
  const STORAGE_KEY = "runner.chat.layout";

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.doUnmock("@tauri-apps/api/window");
    vi.doUnmock("@tauri-apps/api/event");
    vi.doUnmock("./api");
    vi.resetModules();
  });

  it("imports the local v2 payload once and hydrates the node tree", async () => {
    // The env's localStorage is a partial shim; back it with a Map so both
    // the seed write and the freshly-imported module see the same store.
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => void store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
    });

    // The main window persisted a grouped tab; now a secondary window opens.
    localStorage.setItem(
      STORAGE_KEY,
      serializeLayoutSet([applyPresetPure("cols-2", "A", ["A", "B"])], 0),
    );
    vi.resetModules();
    vi.doMock("@tauri-apps/api/window", () => ({
      getCurrentWindow: () => ({ label: "window-secondary" }),
    }));
    vi.doMock("@tauri-apps/api/event", () => ({
      listen: () => Promise.resolve(() => {}),
    }));

    interface MockNode {
      id: string;
      parent_id: string | null;
      position: number;
      type: "project" | "tab" | "mission";
      name: string | null;
      ref_id: string | null;
      layout: string | null;
      pinned_position: number | null;
      last_completed_at: string | null;
      last_viewed_at: string | null;
      created_at: string;
    }
    let nodeRows: MockNode[] = [];
    const sortedRows = () =>
      [...nodeRows].sort((a, b) =>
        (a.parent_id ?? "") < (b.parent_id ?? "")
          ? -1
          : (a.parent_id ?? "") > (b.parent_id ?? "")
            ? 1
            : a.position - b.position,
      );
    const tabUpsert = vi.fn(async () => undefined);
    const nodeDelete = vi.fn(async () => undefined);
    let nextNodeList: Promise<MockNode[]> | null = null;
    const nodeList = vi.fn(() => {
      if (nextNodeList) {
        const response = nextNodeList;
        nextNodeList = null;
        return response;
      }
      return Promise.resolve(sortedRows());
    });
    let nextMoveError: Error | null = null;
    const nodeMove = vi.fn(
      async (id: string, parentId: string | null, orderedIds: string[]) => {
        if (nextMoveError) {
          const error = nextMoveError;
          nextMoveError = null;
          throw error;
        }
        const positions = new Map(
          orderedIds.map((nodeId, position) => [nodeId, position]),
        );
        nodeRows = nodeRows.map((row) => {
          if (row.id === id) {
            return {
              ...row,
              parent_id: parentId,
              position: positions.get(row.id)!,
            };
          }
          const position = positions.get(row.id);
          return position === undefined ? row : { ...row, position };
        });
        return sortedRows();
      },
    );
    vi.doMock("./api", () => ({
      api: {
        node: {
          importOnce: async (
            tabs: { name: string; position: number; layout: string }[],
          ) => {
            nodeRows = tabs.map((tab, index) => ({
              id: `01KTESTTAB0000000000000000${index}`,
              parent_id: null,
              position: tab.position,
              type: "tab" as const,
              name: tab.name,
              ref_id: null,
              layout: tab.layout,
              pinned_position: null,
              last_completed_at: null,
              last_viewed_at: null,
              created_at: "2026-07-12T00:00:00Z",
            }));
            return sortedRows();
          },
          list: nodeList,
          tabUpsert,
          delete: nodeDelete,
          move: nodeMove,
        },
      },
    }));

    const mod = await import("./paneLayout");
    await vi.waitFor(() => {
      expect(mod.getPaneLayouts()[0].id).toContain("01KTESTTAB");
    });

    // Even though a secondary window never WRITES localStorage, it now reads
    // the shared set on cold start, so the tab structure shows immediately.
    const tabs = mod.getPaneLayouts();
    expect(tabs).toHaveLength(1);
    expect(mod.leaves(tabs[0].root).map((l) => l.sessionId)).toEqual([
      "A",
      "B",
    ]);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();

    mod.setGroupNameForSession("A", "Review pair");
    await vi.waitFor(() =>
      expect(tabUpsert).toHaveBeenLastCalledWith(
        expect.objectContaining({ id: tabs[0].id, name: "Review pair" }),
      ),
    );
    expect(mod.getPaneLayout("A").name).toBe("Review pair");

    mod.setGroupNameForSession("A", "   ");
    await vi.waitFor(() =>
      expect(tabUpsert).toHaveBeenLastCalledWith(
        expect.objectContaining({ id: tabs[0].id, name: "" }),
      ),
    );
    expect(mod.getPaneLayout("A").name).toBeNull();
    tabUpsert.mockClear();

    const projectId = "01KTESTPROJECT00000000000000";
    nodeRows.push({
      id: projectId,
      parent_id: null,
      position: 0,
      type: "project",
      name: null,
      ref_id: projectId,
      layout: null,
      pinned_position: null,
      last_completed_at: null,
      last_viewed_at: null,
      created_at: "2026-07-12T00:00:00Z",
    });
    await mod.hydratePaneLayoutsFromDb();

    // A stale in-flight hydration must not clobber the authoritative
    // project move returned by the mutation.
    let releaseStale: (rows: MockNode[]) => void = () => {};
    nextNodeList = new Promise((resolve) => {
      releaseStale = resolve;
    });
    const staleHydration = mod.hydratePaneLayoutsFromDb();
    await vi.waitFor(() => expect(nodeList).toHaveBeenCalled());

    const targetTabId = mod.getPaneLayout("A").id;
    await mod.moveNode(targetTabId, projectId, [targetTabId]);
    releaseStale([]);
    await staleHydration;
    expect(mod.getPaneLayout("A").parentId).toBe(projectId);
    expect(nodeMove).toHaveBeenCalledWith(targetTabId, projectId, [
      targetTabId,
    ]);

    const secondTabId = "01KTESTTAB000000000000000099";
    nodeRows.push({
      id: secondTabId,
      parent_id: projectId,
      position: 1,
      type: "tab",
      name: "C",
      ref_id: null,
      layout: mod.serializeLayout(mod.applyPresetPure("single", "C", ["C"])),
      pinned_position: null,
      last_completed_at: null,
      last_viewed_at: null,
      created_at: "2026-07-12T00:00:01Z",
    });
    await mod.hydratePaneLayoutsFromDb();
    await mod.moveNode(targetTabId, projectId, [
      secondTabId,
      targetTabId,
    ]);
    expect(
      mod
        .getPaneLayouts()
        .filter((layout) => layout.parentId === projectId)
        .map((layout) => layout.id),
    ).toEqual([secondTabId, targetTabId]);
    expect(nodeMove).toHaveBeenCalledWith(targetTabId, projectId, [
      secondTabId,
      targetTabId,
    ]);

    // A rejected move surfaces the error and leaves the local order
    // untouched (the tree only changes on the authoritative response).
    nextMoveError = new Error("write rejected");
    await expect(
      mod.moveNode(targetTabId, projectId, [targetTabId, secondTabId]),
    ).rejects.toThrow("write rejected");
    expect(
      mod
        .getPaneLayouts()
        .filter((layout) => layout.parentId === projectId)
        .map((layout) => layout.id),
    ).toEqual([secondTabId, targetTabId]);

    mod.removeArchivedSessionFromLayout("C");
    mod.removeArchivedSessionFromLayout("A");
    mod.removeArchivedSessionFromLayout("B");
    expect(mod.getPaneLayouts()[0].id).toBe("");
    expect(mod.visibleSessionIds(mod.getPaneLayouts()[0].root)).toEqual([]);
    expect(tabUpsert).not.toHaveBeenCalled();
    expect(nodeDelete).not.toHaveBeenCalled();
  });
});
