/** @vitest-environment jsdom */

// Which source a launch resume's dims come from (impl 0036, decision 2). The
// sizing helpers are stubbed — their pixel math is terminalSizing's own
// concern, and stubbing keeps xterm out of jsdom. What matters here is that a
// laid-out pane wins over an estimate, that a chat session is estimated at its
// own pane's share of THIS launch's window, and that neither path can return
// the previous quit's geometry.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { applyPresetPure, resetPaneLayoutsForTest } from "./paneLayout";

type Grid = { cols: number; rows: number } | null;

const chatPaneAreaBox = vi.fn(() => ({ width: 1000, height: 800 }));
const estimateMissionTerminalGrid = vi.fn<() => Grid>(() => ({
  cols: 90,
  rows: 30,
}));
const terminalGridFromHostElement = vi.fn<(host: HTMLElement) => Grid>(() => ({
  cols: 213,
  rows: 55,
}));
const terminalGridFromPixels = vi.fn<(w: number, h: number) => Grid>(() => ({
  cols: 120,
  rows: 40,
}));

vi.mock("./terminalSizing", () => ({
  TERMINAL_HOST_SESSION_ATTR: "data-terminal-session",
  chatPaneAreaBox: () => chatPaneAreaBox(),
  estimateMissionTerminalGrid: () => estimateMissionTerminalGrid(),
  terminalGridFromHostElement: (host: HTMLElement) =>
    terminalGridFromHostElement(host),
  terminalGridFromPixels: (width: number, height: number) =>
    terminalGridFromPixels(width, height),
}));

const { launchDimsFor } = await import("./launchDims");

beforeEach(() => {
  vi.clearAllMocks();
  document.body.innerHTML = "";
  resetPaneLayoutsForTest([
    applyPresetPure("cols-2", "chat-a", ["chat-a", "chat-b"]),
  ]);
});

afterEach(() => {
  resetPaneLayoutsForTest();
});

describe("launchDimsFor", () => {
  it("measures a laid-out pane rather than estimating", () => {
    document.body.innerHTML =
      '<div data-terminal-session="chat-a"></div>' +
      '<div data-terminal-session="chat-b"></div>';

    expect(launchDimsFor("chat-a")).toEqual({ cols: 213, rows: 55 });
    expect(chatPaneAreaBox).not.toHaveBeenCalled();
    expect(estimateMissionTerminalGrid).not.toHaveBeenCalled();
  });

  it("estimates when the mounted host has no rect to measure", () => {
    document.body.innerHTML = '<div data-terminal-session="chat-a"></div>';
    terminalGridFromHostElement.mockReturnValueOnce(null);

    expect(launchDimsFor("chat-a")).toEqual({ cols: 120, rows: 40 });
    expect(chatPaneAreaBox).toHaveBeenCalledOnce();
  });

  it("estimates a chat session at its own pane's share of the current window", () => {
    expect(launchDimsFor("chat-a")).toEqual({ cols: 120, rows: 40 });
    expect(estimateMissionTerminalGrid).not.toHaveBeenCalled();
    // Half of 1000 across the separator, minus the pane border and header —
    // not the full pane area, and not anything the previous quit persisted.
    expect(terminalGridFromPixels).toHaveBeenCalledWith(999 / 2 - 2, 800 - 36);
  });

  it("tracks a window that changed size between quit and relaunch", () => {
    chatPaneAreaBox.mockReturnValueOnce({ width: 1600, height: 1000 });
    launchDimsFor("chat-a");
    expect(terminalGridFromPixels).toHaveBeenCalledWith(
      1599 / 2 - 2,
      1000 - 36,
    );
  });

  it("uses the mission estimate for a session that is not in any tab", () => {
    expect(launchDimsFor("mission-slot")).toEqual({ cols: 90, rows: 30 });
    expect(chatPaneAreaBox).not.toHaveBeenCalled();
  });
});
