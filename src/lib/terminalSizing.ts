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
