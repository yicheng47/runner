import { describe, expect, it } from "vitest";

import {
  activationResizeRequest,
  isLargeTerminalRowDrop,
  shouldClearViewportBeforePush,
  shouldDelayTerminalResize,
  shouldPushTerminalSize,
  sizePushVerdict,
  terminalSizeAfterDisabledChange,
  terminalSizeAfterRejectedPush,
} from "./terminalResize";

describe("terminal size push state", () => {
  it("retries a rejected push without erasing a newer measurement", () => {
    expect(
      terminalSizeAfterRejectedPush(
        { cols: 120, rows: 30 },
        { cols: 120, rows: 30 },
      ),
    ).toEqual({ cols: 0, rows: 0 });
    expect(
      terminalSizeAfterRejectedPush(
        { cols: 132, rows: 41 },
        { cols: 120, rows: 30 },
      ),
    ).toEqual({ cols: 132, rows: 41 });
  });

  it("pushes current dimensions exactly once after going live", () => {
    const current = { cols: 132, rows: 41 };
    const cleared = terminalSizeAfterDisabledChange(
      current,
      true,
      false,
    );

    expect(cleared).toEqual({ cols: 0, rows: 0 });
    expect(shouldPushTerminalSize(current, cleared)).toBe(true);
    expect(shouldPushTerminalSize(current, current)).toBe(false);
  });

  it("keeps the dedupe state across non-live disabled changes", () => {
    const current = { cols: 132, rows: 41 };

    expect(terminalSizeAfterDisabledChange(current, false, false)).toEqual(
      current,
    );
    expect(terminalSizeAfterDisabledChange(current, false, true)).toEqual(
      current,
    );
  });
});

// #373: exactly one surface — the visible owning pane — may push sizes
// for a session; transitional and non-owning mounts must push nothing.
describe("sizePushVerdict", () => {
  const current = { cols: 209, rows: 59 };
  const lastPushed = { cols: 76, rows: 59 };

  it("lets the visible owning pane push a changed size", () => {
    expect(
      sizePushVerdict({
        current,
        lastPushed,
        active: true,
        resizeDisabled: false,
      }),
    ).toBe("push");
  });

  it("suppresses a transitional pane before ownership", () => {
    expect(
      sizePushVerdict({
        current,
        lastPushed,
        active: true,
        resizeDisabled: true,
      }),
    ).toBe("suppressed-transitional");
    expect(
      sizePushVerdict({
        current,
        lastPushed,
        active: false,
        resizeDisabled: true,
      }),
    ).toBe("suppressed-transitional");
  });

  it("suppresses a non-owning mount (hidden pool, background tab)", () => {
    expect(
      sizePushVerdict({
        current,
        lastPushed,
        active: false,
        resizeDisabled: false,
      }),
    ).toBe("suppressed-nonowner");
  });

  it("keeps the dedupe silent for an unchanged size", () => {
    expect(
      sizePushVerdict({
        current,
        lastPushed: current,
        active: true,
        resizeDisabled: false,
      }),
    ).toBe("unchanged");
    expect(
      sizePushVerdict({
        current,
        lastPushed: current,
        active: false,
        resizeDisabled: false,
      }),
    ).toBe("unchanged");
  });
});

// #373 review finding 1: with the backend debounce, the restoring
// repaint only arrives at settle — so the owner's viewport clear must
// pair 1:1 with pushes that actually go out. A suppressed or deduped
// push clearing the viewport would leave the pane blank with no
// repaint coming.
describe("shouldClearViewportBeforePush", () => {
  it("clears only for an owner push of a clears-on-resize runtime", () => {
    expect(
      shouldClearViewportBeforePush({
        verdict: "push",
        clearsOnResize: true,
        replayJustDrained: false,
        disabled: false,
      }),
    ).toBe(true);
  });

  it("never clears for a suppressed or deduped push", () => {
    for (const verdict of [
      "unchanged",
      "suppressed-transitional",
      "suppressed-nonowner",
    ] as const) {
      expect(
        shouldClearViewportBeforePush({
          verdict,
          clearsOnResize: true,
          replayJustDrained: false,
          disabled: false,
        }),
      ).toBe(false);
    }
  });

  it("keeps the shell / just-replayed / stopped-pane exemptions", () => {
    expect(
      shouldClearViewportBeforePush({
        verdict: "push",
        clearsOnResize: false,
        replayJustDrained: false,
        disabled: false,
      }),
    ).toBe(false);
    expect(
      shouldClearViewportBeforePush({
        verdict: "push",
        clearsOnResize: true,
        replayJustDrained: true,
        disabled: false,
      }),
    ).toBe(false);
    expect(
      shouldClearViewportBeforePush({
        verdict: "push",
        clearsOnResize: true,
        replayJustDrained: false,
        disabled: true,
      }),
    ).toBe(false);
  });
});

describe("activationResizeRequest", () => {
  it("uses a deduped size push after a live hidden-surface return", () => {
    expect(
      activationResizeRequest({
        canvasWasDisplayNone: false,
        wasTransitional: false,
      }),
    ).toEqual({ forceResizeDance: false, pushBackendSize: true });
  });

  it("keeps the resize dance for a display-none pane return", () => {
    expect(
      activationResizeRequest({
        canvasWasDisplayNone: true,
        wasTransitional: false,
      }),
    ).toEqual({ forceResizeDance: true, pushBackendSize: false });
  });

  it("suppresses the resize dance after a transitional resume", () => {
    expect(
      activationResizeRequest({
        canvasWasDisplayNone: true,
        wasTransitional: true,
      }),
    ).toEqual({ forceResizeDance: false, pushBackendSize: true });
  });
});

describe("isLargeTerminalRowDrop", () => {
  it("matches the captured split-pane collapse sizes", () => {
    expect(
      isLargeTerminalRowDrop({ cols: 56, rows: 40 }, { cols: 56, rows: 19 }),
    ).toBe(true);
    expect(
      isLargeTerminalRowDrop({ cols: 46, rows: 18 }, { cols: 46, rows: 7 }),
    ).toBe(true);
  });

  it("does not classify small ordinary resizes as destructive drops", () => {
    expect(
      isLargeTerminalRowDrop({ cols: 100, rows: 40 }, { cols: 100, rows: 36 }),
    ).toBe(false);
    expect(
      isLargeTerminalRowDrop({ cols: 100, rows: 40 }, { cols: 100, rows: 41 }),
    ).toBe(false);
  });
});

describe("shouldDelayTerminalResize", () => {
  it("delays the first destructive row drop for clear-on-resize runtimes", () => {
    expect(
      shouldDelayTerminalResize({
        clearsOnResize: true,
        current: { cols: 56, rows: 40 },
        proposed: { cols: 56, rows: 19 },
        pending: null,
        allowPending: false,
      }),
    ).toBe(true);
  });

  it("keeps duplicate observer events delayed until the stable retry", () => {
    expect(
      shouldDelayTerminalResize({
        clearsOnResize: true,
        current: { cols: 56, rows: 40 },
        proposed: { cols: 56, rows: 19 },
        pending: { cols: 56, rows: 19 },
        allowPending: false,
      }),
    ).toBe(true);
  });

  it("allows the stable retry to apply the same pending dimensions", () => {
    expect(
      shouldDelayTerminalResize({
        clearsOnResize: true,
        current: { cols: 56, rows: 40 },
        proposed: { cols: 56, rows: 19 },
        pending: { cols: 56, rows: 19 },
        allowPending: true,
      }),
    ).toBe(false);
  });

  it("does not delay shell-like runtimes that do not clear on resize", () => {
    expect(
      shouldDelayTerminalResize({
        clearsOnResize: false,
        current: { cols: 56, rows: 40 },
        proposed: { cols: 56, rows: 19 },
        pending: null,
        allowPending: false,
      }),
    ).toBe(false);
  });
});
