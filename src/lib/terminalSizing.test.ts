/** @vitest-environment jsdom */

// pickRespawnDims: the freshness priority behind mission-wide respawn
// sizing (reset / resume-all). The load-bearing property is the ORDER —
// a hidden terminal's measure() returns cached cols that go stale after
// any rail/sidebar/window width change, and a stale-cols respawn
// re-arms the ring purge the sized respawn exists to prevent. The
// cache must therefore lose to every fresh source and never be read
// when a fresh source measures.

import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import type { TerminalGridSize } from "./terminalSizing";

// terminalSizing imports @xterm/xterm at module level; jsdom has no
// canvas, so stub the probe before the module evaluates (same pattern
// as terminalBlank.test.ts).
let pickRespawnDims: typeof import("./terminalSizing").pickRespawnDims;
let chatPaneAreaBox: typeof import("./terminalSizing").chatPaneAreaBox;
let missionPaneAreaBox: typeof import("./terminalSizing").missionPaneAreaBox;
beforeAll(async () => {
  HTMLCanvasElement.prototype.getContext = (() => null) as never;
  ({ pickRespawnDims, chatPaneAreaBox, missionPaneAreaBox } = await import(
    "./terminalSizing"
  ));
});

/** jsdom gives every element a zero rect; give <main> a real one. */
function mountShell(width: number, height: number): void {
  document.body.innerHTML = "<main></main>";
  const main = document.querySelector("main")!;
  main.getBoundingClientRect = () => ({ width, height }) as DOMRect;
}

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

// Surface chrome arithmetic (impl 0036). These feed the launch-resume
// estimate directly, so a stale constant here forks the PTY at the wrong
// size — the failure #363 is about. Asserted on the pixel boxes rather than
// through the grid helpers, which would need a real xterm fit.
describe("pane area boxes", () => {
  beforeEach(() => {
    // Map-backed rather than jsdom's shim, which has no `clear`, so each case
    // starts from "nothing dragged yet".
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
      removeItem: (key: string) => void store.delete(key),
    });
    mountShell(1440, 900);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("takes the chat topbar and the side panel out of the shell box", () => {
    // Nothing dragged yet → the panel's 320px default, not its 200px minimum.
    expect(chatPaneAreaBox()).toEqual({ width: 1440 - 320, height: 900 - 44 });
  });

  it("gives the chat surface the full width once the side panel is closed", () => {
    localStorage.setItem("runner.chat.panel.open", "0");
    expect(chatPaneAreaBox()).toEqual({ width: 1440, height: 900 - 44 });
  });

  it("clamps a dragged side panel to its range", () => {
    localStorage.setItem("runner.chat.panel.width", "9000");
    expect(chatPaneAreaBox().width).toBe(1440 - 480);
    localStorage.setItem("runner.chat.panel.width", "10");
    expect(chatPaneAreaBox().width).toBe(1440 - 200);
  });

  it("takes the mission topbar, tab strip, and rail out of the shell box", () => {
    // h-11 topbar + h-[38px] strip; the rail defaults to 288 un-dragged.
    expect(missionPaneAreaBox()).toEqual({
      width: 1440 - 288,
      height: 900 - 44 - 38,
    });
  });

  it("falls back to the window when the shell has not laid out", () => {
    document.body.innerHTML = "";
    expect(chatPaneAreaBox()).toEqual({
      width: window.innerWidth - 320,
      height: window.innerHeight - 44,
    });
  });
});
