import { describe, expect, it } from "vitest";

import {
  isLargeTerminalRowDrop,
  shouldDelayTerminalResize,
} from "./terminalResize";

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
