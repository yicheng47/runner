/** @vitest-environment jsdom */

import type { Terminal as TerminalType } from "@xterm/xterm";
import { beforeAll, describe, expect, it } from "vitest";

import { replayOutputAtWidths } from "./terminalReplay";

let Terminal: new (opts: object) => TerminalType;

function lines(term: TerminalType): string[] {
  const buffer = term.buffer.active;
  const output: string[] = [];
  for (let index = 0; index < buffer.length; index += 1) {
    const text = buffer.getLine(index)?.translateToString(true) ?? "";
    if (text.length > 0) output.push(text);
  }
  return output;
}

beforeAll(async () => {
  HTMLCanvasElement.prototype.getContext = (() => ({
    measureText: () => ({ width: 8 }),
    fillRect: () => {},
    getImageData: () => ({ data: new Uint8ClampedArray(4) }),
  })) as never;
  if (!window.matchMedia) {
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: () => ({
        matches: false,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    });
  }
  Terminal = (await import("@xterm/xterm")).Terminal as never;
});

describe("width-tagged terminal replay", () => {
  it("replays snapshot and pending live width-runs before final reflow", async () => {
    const term = new Terminal({
      cols: 80,
      rows: 24,
      scrollback: 1000,
      allowProposedApi: true,
    });
    const encode = (text: string) => new TextEncoder().encode(text);
    let takePendingCount = 0;

    await replayOutputAtWidths({
      terminal: term,
      initialChunks: [
        { width: 80, data: encode("a".repeat(120) + "\r\n") },
        { width: 80, data: encode("first-tail\r\n") },
      ],
      takePendingChunks: () => {
        takePendingCount += 1;
        return takePendingCount === 1
          ? [
              { width: 200, data: encode("second-run\r\n") },
              { width: 150, data: encode("third-run\r\n") },
            ]
          : [];
      },
      fallbackWidth: 80,
      target: { cols: 120, rows: 24 },
      setResizeSuppressed: () => {},
      shouldContinue: () => true,
    });

    expect(lines(term)).toEqual([
      "a".repeat(120),
      "first-tail",
      "second-run",
      "third-run",
    ]);
  });

  it("uses the fallback width for legacy chunks and suppresses replay resizes", async () => {
    const term = new Terminal({
      cols: 80,
      rows: 24,
      scrollback: 1000,
      allowProposedApi: true,
    });
    let resizeSuppressed = false;
    let sessionResizeCalls = 0;
    const replayWidths: number[] = [];
    const terminal = {
      resize(cols: number, rows: number) {
        replayWidths.push(cols);
        term.resize(cols, rows);
        if (!resizeSuppressed) sessionResizeCalls += 1;
      },
      write(data: Uint8Array, callback: () => void) {
        term.write(data, callback);
      },
    };

    await replayOutputAtWidths({
      terminal,
      initialChunks: [
        {
          data: new TextEncoder().encode("legacy".repeat(20) + "\r\n"),
        },
      ],
      takePendingChunks: () => [],
      fallbackWidth: 80,
      target: { cols: 120, rows: 24 },
      setResizeSuppressed: (suppressed) => {
        resizeSuppressed = suppressed;
      },
      shouldContinue: () => true,
    });

    expect(lines(term)).toEqual(["legacy".repeat(20)]);
    expect(replayWidths).toEqual([80, 120]);
    expect(sessionResizeCalls).toBe(0);
    terminal.resize(100, 24);
    expect(sessionResizeCalls).toBe(1);
  });
});
