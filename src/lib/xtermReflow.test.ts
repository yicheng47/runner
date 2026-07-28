/** @vitest-environment jsdom */

// Guards xterm's reflow behavior, which the replay design depends on.
//
// Today a session that forks at one width and is displayed at another has
// its entire output ring destroyed (SessionManager::resize's cols-gate),
// because replaying width-coupled bytes into a different-width grid shreds
// them (#306 / impl 0020). The alternative is to replay each width-run at
// the width it was produced at and let xterm reflow the result into the
// pane's grid — no server-side terminal state, no purge.
//
// That rests on xterm reflowing completed lines in both directions,
// including deep scrollback, on the beta we pin (@xterm/xterm
// 6.1.0-beta.220, held for the WebGL atlas fix). If these fail after an
// xterm bump, the replay design is no longer safe — do not just delete
// them.

import { beforeAll, describe, expect, it } from "vitest";
import type { Terminal as TerminalType } from "@xterm/xterm";

// Imported in beforeAll, after the canvas stub below: xterm probes a canvas
// for color metrics while its modules initialize, and a static import would
// hoist above the stub and dump a jsdom stack trace on every run.
let Terminal: new (opts: object) => TerminalType;

/** Resolve once xterm has parsed everything written so far. */
function write(term: TerminalType, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, () => resolve()));
}

/** Every non-empty line of the active buffer, scrollback included. */
function lines(term: TerminalType): string[] {
  const buf = term.buffer.active;
  const out: string[] = [];
  for (let i = 0; i < buf.length; i += 1) {
    const text = buf.getLine(i)?.translateToString(true) ?? "";
    if (text.length > 0) out.push(text);
  }
  return out;
}

function makeTerm(cols: number, rows = 24): TerminalType {
  return new Terminal({ cols, rows, scrollback: 1000, allowProposedApi: true });
}

beforeAll(async () => {
  // Neither affects buffer semantics — stubbed only to keep output clean.
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

describe("xterm reflow", () => {
  it("constructs and writes headlessly (no open())", async () => {
    const term = makeTerm(80);
    await write(term, "hello");
    expect(lines(term)).toEqual(["hello"]);
  });

  // THE LOAD-BEARING ONE. 200 chars at 80 cols hard-wraps into 3 rows;
  // widening to 200 must rejoin them. Without this, replay cannot
  // reconstruct history at a width other than the one it was produced at.
  it("reflows a completed wrapped line when widened", async () => {
    const term = makeTerm(80);
    const text = "a".repeat(200);
    await write(term, text + "\r\n");
    expect(lines(term)).toEqual([
      "a".repeat(80),
      "a".repeat(80),
      "a".repeat(40),
    ]);

    term.resize(200, 24);
    expect(lines(term)).toEqual([text]);
  });

  it("reflows back when narrowed again", async () => {
    const term = makeTerm(80);
    await write(term, "b".repeat(200) + "\r\n");
    term.resize(200, 24);
    term.resize(100, 24);
    expect(lines(term)).toEqual(["b".repeat(100), "b".repeat(100)]);
  });

  // The real-world shape: enough output to reach true scrollback, not just
  // the viewport. Every historical line must reflow, not only recent ones.
  it("reflows deep scrollback, not just the viewport", async () => {
    const term = makeTerm(80);
    for (let i = 0; i < 30; i += 1) {
      await write(term, `line${i}-` + "z".repeat(100) + "\r\n");
    }
    term.resize(200, 24);

    const out = lines(term);
    expect(out).toHaveLength(30);
    expect(out[0]).toBe("line0-" + "z".repeat(100));
    expect(out[29]).toBe("line29-" + "z".repeat(100));
  });

  // The in-progress line the cursor sits on does NOT reflow. That is fine
  // and deliberate to rely on: it is the frame the agent repaints on
  // SIGWINCH anyway. Pinned so a future xterm change here is visible
  // rather than silently altering replay output.
  it("does not reflow the unterminated cursor line", async () => {
    const term = makeTerm(80);
    await write(term, "a".repeat(200)); // no newline — cursor is on it
    term.resize(200, 24);
    expect(lines(term)).toEqual([
      "a".repeat(80),
      "a".repeat(80),
      "a".repeat(40),
    ]);
  });

  // The actual replay shape: several width-runs written in sequence, each
  // at the width it was produced at, then one final resize to the pane.
  // Early runs must survive later resizes, not just the most recent one.
  it("preserves earlier runs across later resizes", async () => {
    const term = makeTerm(80);
    await write(term, "first-run\r\n");
    term.resize(200, 24);
    await write(term, "second-run\r\n");
    term.resize(150, 24);
    await write(term, "third-run\r\n");
    term.resize(120, 24);

    expect(lines(term)).toEqual(["first-run", "second-run", "third-run"]);
  });

  it("reflows a long line written in an early run", async () => {
    const term = makeTerm(80);
    await write(term, "x".repeat(120) + "\r\n");
    await write(term, "tail\r\n");
    term.resize(200, 24);
    expect(lines(term)).toEqual(["x".repeat(120), "tail"]);
  });

  // write() is async. Replay that resizes without awaiting the callback is
  // the obvious way to write this wrong, so pin down whether it corrupts.
  it("resizing without awaiting write() matches the awaited result", async () => {
    const awaited = makeTerm(80);
    await write(awaited, "c".repeat(200) + "\r\n");
    awaited.resize(200, 24);

    const eager = makeTerm(80);
    eager.write("c".repeat(200) + "\r\n");
    eager.resize(200, 24);
    await new Promise((r) => eager.write("", () => r(undefined)));

    expect(lines(eager)).toEqual(lines(awaited));
    expect(lines(eager)).toEqual(["c".repeat(200)]);
  });

  // Alt-screen content is not reflowed by design — the agent repaints on
  // SIGWINCH. What must survive is the primary-buffer scrollback beneath
  // it, which is exactly what the purge destroys today.
  it("keeps primary scrollback across an alt-screen excursion", async () => {
    const term = makeTerm(80);
    await write(term, "scrollback-line\r\n");
    await write(term, "\x1b[?1049h"); // enter alt screen
    await write(term, "TUI frame at 80 cols");
    term.resize(200, 24);
    await write(term, "\x1b[?1049l"); // leave alt screen

    expect(lines(term)).toEqual(["scrollback-line"]);
  });
});
