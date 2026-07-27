// Dims for a session about to be auto-resumed at launch (impl 0036, decision
// 2). Wires the two frontend rungs of the precedence chain to real sources;
// `resolveLaunchDims` (autoResume.ts) owns the ordering, and returning null
// from here is what hands the persisted/default rungs to the backend.
//
// Rung 1 — measured: the session's pane is laid out, so RunnerTerminal's own
// host has the exact box the terminal will fit to. Only reachable when the
// surface happens to be mounted; at a cold launch the main window boots on
// /runners and nothing is.
//
// Rung 2 — estimated: derived from THIS launch's window. Which chrome to
// subtract depends on the surface the session returns into, so chat sessions
// (identified by membership in a persisted tab layout — only direct chats
// live in tabs) get the chat pane area divided by their pane's share of the
// split, and everything else gets the mission estimate.

import {
  getPaneLayouts,
  leafForSession,
  paneBoxForSession,
  type PaneLayout,
} from "./paneLayout";
import { resolveLaunchDims } from "./autoResume";
import {
  chatPaneAreaBox,
  estimateMissionTerminalGrid,
  TERMINAL_HOST_SESSION_ATTR,
  terminalGridFromHostElement,
  terminalGridFromPixels,
  type TerminalGridSize,
} from "./terminalSizing";

function measureMountedTerminal(sessionId: string): TerminalGridSize | null {
  // Matched by attribute value rather than interpolated into the selector:
  // session ids are opaque, and a selector that throws would degrade to the
  // estimate silently.
  const host = Array.from(
    document.querySelectorAll<HTMLElement>(`[${TERMINAL_HOST_SESSION_ATTR}]`),
  ).find((el) => el.getAttribute(TERMINAL_HOST_SESSION_ATTR) === sessionId);
  return host ? terminalGridFromHostElement(host) : null;
}

function tabLayoutFor(sessionId: string): PaneLayout | null {
  return (
    getPaneLayouts().find((layout) => leafForSession(layout.root, sessionId)) ??
    null
  );
}

function estimateForSession(sessionId: string): TerminalGridSize | null {
  const layout = tabLayoutFor(sessionId);
  if (!layout) return estimateMissionTerminalGrid();
  const box = paneBoxForSession(layout, sessionId, chatPaneAreaBox());
  return box ? terminalGridFromPixels(box.width, box.height) : null;
}

export function launchDimsFor(sessionId: string): TerminalGridSize | null {
  return resolveLaunchDims({
    measure: () => measureMountedTerminal(sessionId),
    estimate: () => estimateForSession(sessionId),
  });
}
