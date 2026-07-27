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

// Must match `py-3 pl-3 pr-1` on both terminal host surfaces.
const TERMINAL_FRAME_PADDING_PX = 12;
const TERMINAL_FRAME_RIGHT_PADDING_PX = 4;
export const TERMINAL_SCROLLBAR_WIDTH_PX = 8;

/** Attribute RunnerTerminal stamps on its host element, keyed by session id.
 *  Must match the literal in that component's JSX. */
export const TERMINAL_HOST_SESSION_ATTR = "data-terminal-session";

// Mission-surface chrome above the pane container, mirroring
// MissionWorkspace: an `h-11` topbar and an `h-[38px]` slot-tab strip. Both
// are border-box, so each value is the full cost.
const MISSION_HEADER_HEIGHT_PX = 44;
const MISSION_TAB_STRIP_HEIGHT_PX = 38;
const MISSION_RAIL_DEFAULT_WIDTH_PX = 288;
const MISSION_RAIL_MIN_WIDTH_PX = 200;
const MISSION_RAIL_MAX_WIDTH_PX = 480;

// Chat-surface chrome, mirroring RunnerChat: an `h-11` topbar over the pane
// area, and a right side panel whose open state and width live in
// localStorage. Keys are duplicated from RunnerChat the same way the mission
// rail's are duplicated from MissionWorkspace above — importing either page
// here would close an import cycle.
const CHAT_HEADER_HEIGHT_PX = 44;
const CHAT_PANEL_DEFAULT_WIDTH_PX = 320;
const CHAT_PANEL_MIN_WIDTH_PX = 200;
const CHAT_PANEL_MAX_WIDTH_PX = 480;

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
    scrollback: 1,
    scrollbar: { width: TERMINAL_SCROLLBAR_WIDTH_PX },
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
): TerminalGridSize | null {
  const width = Math.max(
    0,
    widthPx - TERMINAL_FRAME_PADDING_PX - TERMINAL_FRAME_RIGHT_PADDING_PX,
  );
  const height = Math.max(0, heightPx - TERMINAL_FRAME_PADDING_PX * 2);
  if (width <= 0 || height <= 0) return null;
  return fitTemporaryTerminal(width, height);
}

export function terminalGridFromElement(
  container: HTMLElement,
): TerminalGridSize | null {
  const rect = container.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return terminalGridFromPixels(rect.width, rect.height);
}

/**
 * Grid for a host that already sits INSIDE the padding frame — RunnerTerminal
 * owns exactly that box, while `terminalGridFromElement` expects the padded
 * wrapper around it. A display:none host has no rect and returns null, which
 * is the desired answer at launch: an estimate from the live window beats a
 * hidden pane's last-visible geometry.
 */
export function terminalGridFromHostElement(
  host: HTMLElement,
): TerminalGridSize | null {
  const rect = host.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return fitTemporaryTerminal(rect.width, rect.height);
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

/** The shell's <main> box — the area every terminal-bearing surface renders
 *  into. Mounted from boot, so this reads true even with no chat or mission
 *  surface on screen. */
function shellContentBox(): { width: number; height: number } {
  const main = document.querySelector("main");
  const rect = main?.getBoundingClientRect();
  return {
    width: rect && rect.width > 0 ? rect.width : window.innerWidth,
    height: rect && rect.height > 0 ? rect.height : window.innerHeight,
  };
}

/** Width a collapsible side surface (mission rail, chat side panel) is taking
 *  out of <main> right now: 0 when closed, else its stored width clamped to
 *  the drag range, or the default when nothing was ever dragged. */
function storedSideWidth(
  openKey: string,
  widthKey: string,
  defaultWidth: number,
  minWidth: number,
  maxWidth: number,
): number {
  try {
    if (localStorage.getItem(openKey) === "0") return 0;
    const raw = Number(localStorage.getItem(widthKey));
    const width = Number.isFinite(raw) && raw > 0 ? raw : defaultWidth;
    return Math.min(maxWidth, Math.max(minWidth, width));
  } catch {
    return defaultWidth;
  }
}

/**
 * Pixel box the mission workspace's pane container occupies right now:
 * <main> minus the topbar, the slot-tab strip, and the runners rail. The
 * workspace shows one slot terminal at a time, so unlike the chat surface
 * there is no split to divide this by.
 */
export function missionPaneAreaBox(): { width: number; height: number } {
  const { width, height } = shellContentBox();
  const railWidth = storedSideWidth(
    "runner.mission.rail.open",
    "runner.mission.rail.width",
    MISSION_RAIL_DEFAULT_WIDTH_PX,
    MISSION_RAIL_MIN_WIDTH_PX,
    MISSION_RAIL_MAX_WIDTH_PX,
  );
  return {
    width: Math.max(0, width - railWidth),
    height: Math.max(
      0,
      height - MISSION_HEADER_HEIGHT_PX - MISSION_TAB_STRIP_HEIGHT_PX,
    ),
  };
}

export function estimateMissionTerminalGrid(): TerminalGridSize | null {
  const { width, height } = missionPaneAreaBox();
  return terminalGridFromPixels(width, height);
}

/**
 * Pixel box the chat surface's pane area occupies right now: <main> minus the
 * chat topbar and the side panel. This is the area ChatPaneGroup divides
 * between panes, so a session's own box is this run through
 * `paneBoxForSession` (paneLayout.ts) — the split divisor is not folded in
 * here, because the same area serves every pane of a tab.
 */
export function chatPaneAreaBox(): { width: number; height: number } {
  const { width, height } = shellContentBox();
  const panelWidth = storedSideWidth(
    "runner.chat.panel.open",
    "runner.chat.panel.width",
    CHAT_PANEL_DEFAULT_WIDTH_PX,
    CHAT_PANEL_MIN_WIDTH_PX,
    CHAT_PANEL_MAX_WIDTH_PX,
  );
  return {
    width: Math.max(0, width - panelWidth),
    height: Math.max(0, height - CHAT_HEADER_HEIGHT_PX),
  };
}
