/** @vitest-environment jsdom */

// pickRespawnDims: the freshness priority behind mission-wide respawn
// sizing (reset / resume-all). The load-bearing property is the ORDER —
// a hidden terminal's measure() returns cached cols that go stale after
// any rail/sidebar/window width change, and a stale-cols respawn
// re-arms the ring purge the sized respawn exists to prevent. The
// cache must therefore lose to every fresh source and never be read
// when a fresh source measures.

import { beforeAll, describe, expect, it, vi } from "vitest";

import type { TerminalGridSize } from "./terminalSizing";

// terminalSizing imports @xterm/xterm at module level; jsdom has no
// canvas, so stub the probe before the module evaluates (same pattern
// as terminalBlank.test.ts).
let pickRespawnDims: typeof import("./terminalSizing").pickRespawnDims;
beforeAll(async () => {
  HTMLCanvasElement.prototype.getContext = (() => null) as never;
  ({ pickRespawnDims } = await import("./terminalSizing"));
});

const dims = (cols: number): TerminalGridSize => ({ cols, rows: 40 });

describe("pickRespawnDims", () => {
  it("prefers the active slot's fresh fit over everything", () => {
    const probe = vi.fn(() => dims(150));
    const cache = vi.fn(() => dims(120));
    expect(
      pickRespawnDims({
        measureActiveSlot: () => dims(170),
        probeContainer: probe,
        readHiddenCache: cache,
      }),
    ).toEqual(dims(170));
    expect(probe).not.toHaveBeenCalled();
    expect(cache).not.toHaveBeenCalled();
  });

  it("prefers the container probe over a stale hidden cache", () => {
    // Feed tab active after a layout change: the probe reads the
    // CURRENT rect, the hidden cache still holds pre-change cols.
    const cache = vi.fn(() => dims(120));
    expect(
      pickRespawnDims({
        measureActiveSlot: () => null,
        probeContainer: () => dims(150),
        readHiddenCache: cache,
      }),
    ).toEqual(dims(150));
    expect(cache).not.toHaveBeenCalled();
  });

  it("falls back to the hidden cache only when nothing else measures", () => {
    expect(
      pickRespawnDims({
        measureActiveSlot: () => null,
        probeContainer: () => null,
        readHiddenCache: () => dims(120),
      }),
    ).toEqual(dims(120));
  });

  it("returns null when no source measures", () => {
    expect(
      pickRespawnDims({
        measureActiveSlot: () => null,
        probeContainer: () => null,
        readHiddenCache: () => null,
      }),
    ).toBeNull();
  });
});
