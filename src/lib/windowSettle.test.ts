// The settle gate's contract: it resolves as soon as the webview viewport
// agrees with the restored native frame, it never waits past the ceiling, and
// it degrades to "proceed anyway" rather than hanging when there is nothing to
// compare against (impl 0036, phase 1).

import { describe, expect, it, vi } from "vitest";

import {
  awaitWindowGeometrySettle,
  windowSettleDeps,
  WINDOW_SETTLE_TOLERANCE_PX,
  type WindowSettleDeps,
} from "./windowSettle";

/** Deps over a fake clock: each `step` advances time by `stepMs`. */
function harness(
  overrides: Partial<WindowSettleDeps> & { stepMs?: number } = {},
): { deps: WindowSettleDeps; step: ReturnType<typeof vi.fn> } {
  let clock = 0;
  const stepMs = overrides.stepMs ?? 32;
  const step = vi.fn(async () => {
    clock += stepMs;
  });
  return {
    step,
    deps: {
      nativeLogicalWidth: async () => 1440,
      viewportLogicalWidth: () => 1440,
      step,
      now: () => clock,
      ...overrides,
    },
  };
}

describe("awaitWindowGeometrySettle", () => {
  it("settles without waiting when the viewport already matches", async () => {
    const { deps, step } = harness();
    await expect(awaitWindowGeometrySettle(deps)).resolves.toBe("settled");
    expect(step).not.toHaveBeenCalled();
  });

  it("waits for a lagging viewport to catch up with the restored frame", async () => {
    // The window was restored wider than the frame the webview laid out at;
    // reading geometry before the third poll would give the old width.
    const widths = [640, 640, 1440];
    const viewportLogicalWidth = vi.fn(() => widths.shift() ?? 1440);
    const { deps, step } = harness({ viewportLogicalWidth });

    await expect(awaitWindowGeometrySettle(deps)).resolves.toBe("settled");
    expect(step).toHaveBeenCalledTimes(2);
  });

  it("accepts a difference within the rounding tolerance", async () => {
    const { deps } = harness({
      viewportLogicalWidth: () => 1440 - WINDOW_SETTLE_TOLERANCE_PX,
    });
    await expect(awaitWindowGeometrySettle(deps)).resolves.toBe("settled");
  });

  it("gives up at the ceiling instead of hanging", async () => {
    const { deps, step } = harness({
      viewportLogicalWidth: () => 640,
      ceilingMs: 100,
      stepMs: 32,
    });
    await expect(awaitWindowGeometrySettle(deps)).resolves.toBe("timeout");
    // 0/32/64/96 compared, 128 past the deadline.
    expect(step).toHaveBeenCalledTimes(4);
  });

  it("reports unavailable when the native frame cannot be read", async () => {
    const thrown = harness({
      nativeLogicalWidth: async () => {
        throw new Error("no tauri runtime");
      },
    });
    await expect(awaitWindowGeometrySettle(thrown.deps)).resolves.toBe(
      "unavailable",
    );
    expect(thrown.step).not.toHaveBeenCalled();

    const empty = harness({ nativeLogicalWidth: async () => null });
    await expect(awaitWindowGeometrySettle(empty.deps)).resolves.toBe(
      "unavailable",
    );
  });
});

// The unit conversion is the part that can silently never converge. WebKit
// keeps devicePixelRatio at the display scale while page zoom moves
// innerWidth, so zoom has to come from the setting — inferring it from DPR
// would leave every zoomed launch waiting out the ceiling.
describe("windowSettleDeps", () => {
  function deps(scaleFactor: number, zoom: number, cssWidth: number) {
    return windowSettleDeps({
      nativeInnerWidthPx: async () => 2880,
      nativeScaleFactor: async () => scaleFactor,
      viewportCssWidth: () => cssWidth,
      appZoom: () => zoom,
    });
  }

  it("settles at 100% zoom on a 2x display", async () => {
    // 2880 physical / 2 = 1440 logical; the viewport reports it 1:1.
    await expect(
      awaitWindowGeometrySettle({ ...deps(2, 1.0, 1440), ceilingMs: 0 }),
    ).resolves.toBe("settled");
  });

  it("settles at a non-100% zoom, where the viewport is in zoomed CSS pixels", async () => {
    // Same 1440-point window at 125%: the page lays out at 1152 CSS px.
    // ceilingMs 0 makes this fail loudly rather than converge by waiting.
    await expect(
      awaitWindowGeometrySettle({ ...deps(2, 1.25, 1152), ceilingMs: 0 }),
    ).resolves.toBe("settled");
  });

  it("does not treat a stale pre-restore viewport as settled", async () => {
    await expect(
      awaitWindowGeometrySettle({ ...deps(2, 1.25, 640), ceilingMs: 0 }),
    ).resolves.toBe("timeout");
  });

  it("reports unavailable when the scale factor is unusable", async () => {
    await expect(
      awaitWindowGeometrySettle({ ...deps(0, 1.0, 1440), ceilingMs: 0 }),
    ).resolves.toBe("unavailable");
  });
});
