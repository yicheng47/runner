import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import {
  readTerminalFontFamily,
  readTerminalFontSize,
  resolveTerminalFontStack,
} from "./settings";

export interface TerminalGridSize {
  cols: number;
  rows: number;
}

export const SLOT_TERMINAL_FRAME_PADDING_PX = 12;

const MISSION_HEADER_HEIGHT_PX = 88;
const MISSION_TAB_STRIP_HEIGHT_PX = 38;
const MISSION_RAIL_DEFAULT_WIDTH_PX = 288;
const MISSION_RAIL_MIN_WIDTH_PX = 200;
const MISSION_RAIL_MAX_WIDTH_PX = 480;

function fitTemporaryTerminal(widthPx: number, heightPx: number): TerminalGridSize | null {
  const fontFamily = resolveTerminalFontStack(readTerminalFontFamily());
  const fontSize = readTerminalFontSize();
  const host = document.createElement("div");
  host.style.position = "absolute";
  host.style.left = "-10000px";
  host.style.top = "-10000px";
  host.style.width = `${widthPx}px`;
  host.style.height = `${heightPx}px`;
  host.style.visibility = "hidden";
  host.style.pointerEvents = "none";
  document.body.appendChild(host);

  const term = new Terminal({
    cols: 80,
    rows: 24,
    fontFamily,
    fontSize,
    scrollback: 0,
  });
  const fit = new FitAddon();
  try {
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    return { cols: term.cols, rows: term.rows };
  } finally {
    term.dispose();
    document.body.removeChild(host);
  }
}

export function terminalGridFromPixels(
  widthPx: number,
  heightPx: number,
  framePaddingPx = SLOT_TERMINAL_FRAME_PADDING_PX,
): TerminalGridSize | null {
  const width = Math.max(0, widthPx - framePaddingPx * 2);
  const height = Math.max(0, heightPx - framePaddingPx * 2);
  if (width <= 0 || height <= 0) return null;
  return fitTemporaryTerminal(width, height);
}

export function terminalGridFromElement(
  container: HTMLElement,
  framePaddingPx = SLOT_TERMINAL_FRAME_PADDING_PX,
): TerminalGridSize | null {
  const rect = container.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return terminalGridFromPixels(rect.width, rect.height, framePaddingPx);
}

/**
 * Source priority for the dims passed to a mission-wide respawn
 * (reset / resume-all). Freshness beats availability:
 *
 *   1. The active slot tab's terminal — its pane is visible, so
 *      `measure()` runs a real fit against the current layout.
 *   2. The pane-container probe — reads the container's CURRENT rect
 *      (covers the feed tab, where no slot terminal is visible).
 *   3. A hidden terminal's cached last-fit dims. display:none panes
 *      can't fit, so `measure()` returns whatever cols the pane had
 *      when it was last visible — stale after any rail/sidebar/window
 *      width change, and respawning at stale cols re-arms the ring
 *      purge the sized respawn exists to prevent. Last resort only.
 *
 * Sources are thunks so losing tiers aren't computed (the container
 * probe opens a throwaway xterm).
 */
export function pickRespawnDims(sources: {
  measureActiveSlot: () => TerminalGridSize | null;
  probeContainer: () => TerminalGridSize | null;
  readHiddenCache: () => TerminalGridSize | null;
}): TerminalGridSize | null {
  return (
    sources.measureActiveSlot() ??
    sources.probeContainer() ??
    sources.readHiddenCache()
  );
}

export function estimateMissionTerminalGrid(): TerminalGridSize | null {
  const main = document.querySelector("main");
  const rect = main?.getBoundingClientRect();
  const width = rect && rect.width > 0 ? rect.width : window.innerWidth;
  const height = rect && rect.height > 0 ? rect.height : window.innerHeight;

  let railWidth = 0;
  try {
    if (localStorage.getItem("runner.mission.rail.open") !== "0") {
      const raw = Number(localStorage.getItem("runner.mission.rail.width"));
      railWidth = Number.isFinite(raw) ? raw : MISSION_RAIL_DEFAULT_WIDTH_PX;
      railWidth = Math.min(
        MISSION_RAIL_MAX_WIDTH_PX,
        Math.max(MISSION_RAIL_MIN_WIDTH_PX, railWidth),
      );
    }
  } catch {
    railWidth = MISSION_RAIL_DEFAULT_WIDTH_PX;
  }

  return terminalGridFromPixels(
    Math.max(0, width - railWidth),
    Math.max(0, height - MISSION_HEADER_HEIGHT_PX - MISSION_TAB_STRIP_HEIGHT_PX),
  );
}
