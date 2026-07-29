/** @vitest-environment jsdom */

// Placement gate for the flat terminal stack (#373 defect 2): a wrapper
// that paneGeo has not yet positioned from a real pane-body rect must
// stay at 0×0 — otherwise the absolute wrapper shrink-wraps xterm's
// default 80×24 canvas into a measurable ~78×23 box and that garbage
// fit gets pushed to the PTY, purging the ring of a ~209-col session.

import { beforeEach, describe, expect, it, vi } from "vitest";

import { createPaneGeometry } from "./paneGeometry";
import type { PaneNode } from "./paneLayout";

const leaf = (id: string, sessionId: string | null): PaneNode => ({
  kind: "leaf",
  id,
  sessionId,
});

function elementWithRect(rect: {
  left: number;
  top: number;
  width: number;
  height: number;
}): HTMLDivElement {
  const el = document.createElement("div");
  el.getBoundingClientRect = () =>
    ({
      ...rect,
      right: rect.left + rect.width,
      bottom: rect.top + rect.height,
      x: rect.left,
      y: rect.top,
      toJSON: () => ({}),
    }) as DOMRect;
  return el;
}

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

describe("createPaneGeometry placement gate", () => {
  it("holds a wrapper at 0x0 until its pane body can place it", () => {
    const geo = createPaneGeometry();
    geo.containerRef(
      elementWithRect({ left: 0, top: 0, width: 1400, height: 900 }),
    );
    geo.setRoot(leaf("p1", "s1"));

    // Wrapper attaches before any pane body is registered — the
    // restored-split hydration window.
    const wrap = document.createElement("div");
    geo.termWrapRefFor("s1")(wrap);
    expect(wrap.style.width).toBe("0px");
    expect(wrap.style.height).toBe("0px");

    // Pane body registers with a real rect; sync places the wrapper.
    geo.paneBodyRefFor("p1")(
      elementWithRect({ left: 10, top: 20, width: 1200, height: 800 }),
    );
    geo.sync();
    expect(wrap.style.width).toBe("1200px");
    expect(wrap.style.height).toBe("800px");
    expect(wrap.style.left).toBe("10px");
    expect(wrap.style.top).toBe("20px");
  });

  it("does not place a wrapper from a zero-rect pane body", () => {
    const geo = createPaneGeometry();
    geo.containerRef(
      elementWithRect({ left: 0, top: 0, width: 1400, height: 900 }),
    );
    geo.setRoot(leaf("p1", "s1"));

    const wrap = document.createElement("div");
    geo.termWrapRefFor("s1")(wrap);
    geo.paneBodyRefFor("p1")(
      elementWithRect({ left: 0, top: 0, width: 0, height: 0 }),
    );
    geo.sync();
    expect(wrap.style.width).toBe("0px");
    expect(wrap.style.height).toBe("0px");
  });

  it("re-gates a wrapper that detaches and reattaches", () => {
    const geo = createPaneGeometry();
    geo.containerRef(
      elementWithRect({ left: 0, top: 0, width: 1400, height: 900 }),
    );
    geo.setRoot(leaf("p1", "s1"));
    geo.paneBodyRefFor("p1")(
      elementWithRect({ left: 0, top: 0, width: 1200, height: 800 }),
    );

    const wrap = document.createElement("div");
    geo.termWrapRefFor("s1")(wrap);
    expect(wrap.style.width).toBe("1200px");

    // Detach (session archived / chat closed), then a fresh wrapper
    // attaches for the same session while its pane body is gone.
    geo.termWrapRefFor("s1")(null);
    geo.paneBodyRefFor("p1")(null);
    const wrap2 = document.createElement("div");
    geo.termWrapRefFor("s1")(wrap2);
    expect(wrap2.style.width).toBe("0px");
    expect(wrap2.style.height).toBe("0px");
  });
});
