// Dims for a session about to be auto-resumed at launch (impl 0038, decision
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
import { logLaunchDims } from "./frontendLog";
import {
  chatPaneAreaBox,
  estimateMissionTerminalGrid,
  missionPaneAreaBox,
  shellContentBox,
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

function estimateForSession(
  sessionId: string,
  detail: string[],
): TerminalGridSize | null {
  const shell = shellContentBox();
  detail.push(
    shell
      ? `main=${Math.round(shell.width)}x${Math.round(shell.height)}`
      : "main=null",
  );
  const layout = tabLayoutFor(sessionId);
  if (!layout) {
    detail.push("surface=mission");
    const area = missionPaneAreaBox();
    if (shell && area) {
      detail.push(`rail=${Math.round(shell.width - area.width)}px`);
    }
    return estimateMissionTerminalGrid();
  }
  detail.push("surface=chat");
  const area = chatPaneAreaBox();
  if (!area) return null;
  if (shell) detail.push(`panel=${Math.round(shell.width - area.width)}px`);
  const box = paneBoxForSession(layout, sessionId, area);
  if (!box) return null;
  detail.push(`paneShare=${(box.width / area.width).toFixed(2)}`);
  return terminalGridFromPixels(box.width, box.height);
}

export function launchDimsFor(sessionId: string): TerminalGridSize | null {
  // The detail trail exists for the #366 file log: one line per resumed
  // session showing which rung answered and the inputs the estimate saw.
  const detail: string[] = [];
  const dims = resolveLaunchDims({
    measure: () => {
      const measured = measureMountedTerminal(sessionId);
      if (measured) detail.push("rung=measured");
      return measured;
    },
    estimate: () => {
      detail.push("rung=estimate");
      return estimateForSession(sessionId, detail);
    },
  });
  logLaunchDims(
    `session=${sessionId} ${detail.join(" ")} -> ` +
      (dims ? `${dims.cols}x${dims.rows}` : "null (backend persisted/default rung)"),
  );
  return dims;
}
