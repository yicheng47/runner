/** @vitest-environment jsdom */

// terminalGridIsBlank + blankDanceDecision (#312): the "blank grid
// must dance" predicate and its escalation decision. Runs against a
// real xterm Terminal (unopened — buffer + parser work without a DOM
// attach) so the tests cover the actual buffer semantics:
// translateToString trimming, alt-screen buffers, the scrollback
// shortcut, and the async write queue behind the defer decision.

import type { Terminal as XtermTerminal } from "@xterm/xterm";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import {
  blankDanceDecision,
  createBlankRecheckGate,
  terminalGridIsBlank,
} from "./terminalBlank";

// jsdom has no canvas: xterm's color helpers probe
// HTMLCanvasElement.getContext at module load and jsdom logs a noisy
// (harmless) "Not implemented" error per run. Stub the probe before
// xterm evaluates — it takes the same fallback as a failed probe,
// minus the stderr noise. Hence the dynamic import; the type-only
// import above is erased at compile time and loads nothing.
let Terminal: typeof XtermTerminal;
beforeAll(async () => {
  HTMLCanvasElement.prototype.getContext = (() => null) as never;
  ({ Terminal } = await import("@xterm/xterm"));
});

const write = (t: XtermTerminal, data: string) =>
  new Promise<void>((resolve) => t.write(data, resolve));

let term: XtermTerminal | undefined;
const makeTerm = () => {
  term = new Terminal({ cols: 80, rows: 24, allowProposedApi: true });
  return term;
};
afterEach(() => {
  term?.dispose();
  term = undefined;
});

describe("terminalGridIsBlank", () => {
  it("is blank after construction", () => {
    expect(terminalGridIsBlank(makeTerm())).toBe(true);
  });

  it("is blank after reset with zero bytes replayed", async () => {
    const t = makeTerm();
    await write(t, "some prior frame");
    t.reset();
    expect(terminalGridIsBlank(t)).toBe(true);
  });

  it("is not blank once any glyph is on the grid", async () => {
    const t = makeTerm();
    await write(t, "x");
    expect(terminalGridIsBlank(t)).toBe(false);
  });

  it("treats cursor-movement-only output as blank", async () => {
    const t = makeTerm();
    // Home + move around, no printable glyphs — visually still empty.
    await write(t, "\x1b[H\x1b[10;10H");
    expect(terminalGridIsBlank(t)).toBe(true);
  });

  it("sees content painted on the alt screen", async () => {
    const t = makeTerm();
    await write(t, "\x1b[?1049h");
    expect(terminalGridIsBlank(t)).toBe(true);
    await write(t, "\x1b[2;4HTUI frame");
    expect(terminalGridIsBlank(t)).toBe(false);
  });

  it("counts scrollback as content even when the viewport is clear", async () => {
    const t = makeTerm();
    // Fill past the viewport so lines rotate into scrollback, then
    // erase the visible region — retained history means the pane is
    // NOT blank, and the transitional latch keeps its guarantee.
    await write(t, Array.from({ length: 30 }, (_, i) => `line ${i}`).join("\r\n"));
    await write(t, "\x1b[2J\x1b[H");
    expect(terminalGridIsBlank(t)).toBe(false);
  });
});

describe("blankDanceDecision", () => {
  it("dances on a settled blank grid", () => {
    expect(blankDanceDecision(makeTerm(), 0)).toBe("dance");
  });

  it("defers while live bytes are queued but unparsed", async () => {
    const t = makeTerm();
    // NOT awaited — xterm parses asynchronously, so the buffer still
    // reads blank right after write() returns. This is the queued-
    // output race: without the pending-write count the stale read
    // would dance over an incoming repaint, defeating the
    // transitional latch's double-repaint guard.
    const flushed = write(t, "x");
    expect(terminalGridIsBlank(t)).toBe(true);
    expect(blankDanceDecision(t, 1)).toBe("defer");
    // After the flush the deferred re-check must see the content and
    // fall back to the plain-push path.
    await flushed;
    expect(blankDanceDecision(t, 0)).toBe("none");
  });

  it("does not dance once the grid holds content", async () => {
    const t = makeTerm();
    await write(t, "x");
    expect(blankDanceDecision(t, 0)).toBe("none");
  });

  it("still dances after a flush that parsed no visible glyphs", async () => {
    const t = makeTerm();
    // Control-only bytes (cursor moves) flush without painting — the
    // re-check after flush must escalate, not stall forever.
    await write(t, "\x1b[H\x1b[5;5H");
    expect(blankDanceDecision(t, 0)).toBe("dance");
  });
});

describe("createBlankRecheckGate", () => {
  it("coalesces two defers across a control-only flush into one dancing recheck", async () => {
    // The reviewer scenario: overlapping refresh passes (an ordinary
    // wake invokes its refit twice) each observe blank+pending against
    // the SAME in-flight write. The gate must yield exactly one
    // recheck at flush — not one dance per observation. Mirrors
    // writeOutput's production shape: beginWrite before t.write,
    // endWrite in its parse callback.
    const t = makeTerm();
    const gate = createBlankRecheckGate();
    let rechecks = 0;
    const flushed = new Promise<void>((resolve) => {
      gate.beginWrite();
      t.write("\x1b[H\x1b[5;5H", () => {
        if (gate.endWrite()) rechecks += 1;
        resolve();
      });
    });
    expect(blankDanceDecision(t, gate.pendingWrites())).toBe("defer");
    gate.requestRecheck();
    expect(blankDanceDecision(t, gate.pendingWrites())).toBe("defer");
    gate.requestRecheck();
    await flushed;
    expect(rechecks).toBe(1);
    // The single recheck sees a settled, still-blank grid: one dance.
    expect(blankDanceDecision(t, gate.pendingWrites())).toBe("dance");
  });

  it("rechecks only once the queue fully drains", () => {
    const gate = createBlankRecheckGate();
    gate.beginWrite();
    gate.beginWrite();
    gate.requestRecheck();
    expect(gate.endWrite()).toBe(false);
    expect(gate.endWrite()).toBe(true);
  });

  it("does not recheck without a request", () => {
    const gate = createBlankRecheckGate();
    gate.beginWrite();
    expect(gate.endWrite()).toBe(false);
  });

  it("a flush consumes the request — the next flush is quiet", () => {
    const gate = createBlankRecheckGate();
    gate.beginWrite();
    gate.requestRecheck();
    expect(gate.endWrite()).toBe(true);
    gate.beginWrite();
    expect(gate.endWrite()).toBe(false);
  });

  it("cancelRecheck drops a stale request (session swap)", () => {
    const gate = createBlankRecheckGate();
    gate.beginWrite();
    gate.requestRecheck();
    gate.cancelRecheck();
    expect(gate.endWrite()).toBe(false);
  });
});
