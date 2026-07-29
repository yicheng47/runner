export interface TerminalGridSize {
  cols: number;
  rows: number;
}

export function shouldPushTerminalSize(
  current: TerminalGridSize,
  lastPushed: TerminalGridSize,
): boolean {
  return (
    current.cols !== lastPushed.cols || current.rows !== lastPushed.rows
  );
}

export type SizePushVerdict =
  | "push"
  | "unchanged"
  | "suppressed-transitional"
  | "suppressed-nonowner";

/**
 * Single gate for every backend size push (#373). Exactly one surface —
 * the visible owning pane (`active`) — may write sizes for a session;
 * hidden-pool wrappers, background tabs, and other keep-alive mounts
 * refit their local xterm but observe without writing, which kills the
 * two-writer oscillation (#373 defect 3, the #313 class). Transitional
 * panes (resume/start in flight) stay suppressed as before. `unchanged`
 * keeps the existing dedupe silent — it is not a suppression.
 */
export function sizePushVerdict({
  current,
  lastPushed,
  active,
  resizeDisabled,
}: {
  current: TerminalGridSize;
  lastPushed: TerminalGridSize;
  active: boolean;
  resizeDisabled: boolean;
}): SizePushVerdict {
  if (!shouldPushTerminalSize(current, lastPushed)) return "unchanged";
  if (resizeDisabled) return "suppressed-transitional";
  if (!active) return "suppressed-nonowner";
  return "push";
}

/**
 * Whether the owner pane should hard-clear its visible region before a
 * backend size push. The clear prevents the stacking artifact when the
 * settle-driven SIGWINCH repaint lands, so it must pair 1:1 with pushes
 * that can produce such a repaint: a suppressed or deduped push must
 * never clear (a mount that doesn't write may not blank its viewport —
 * with the backend debounce, no repaint would follow to restore it).
 * The backend guarantees the other half of the pairing: a purge settle
 * repaints via the real width change, and a round-trip settle forces
 * the repaint with the rows nudge (see `settle_pending_resize`).
 */
export function shouldClearViewportBeforePush({
  verdict,
  clearsOnResize,
  replayJustDrained,
  disabled,
}: {
  verdict: SizePushVerdict;
  clearsOnResize: boolean;
  replayJustDrained: boolean;
  disabled: boolean;
}): boolean {
  return (
    verdict === "push" && clearsOnResize && !replayJustDrained && !disabled
  );
}

export function terminalSizeAfterRejectedPush(
  lastPushed: TerminalGridSize,
  rejected: TerminalGridSize,
): TerminalGridSize {
  if (!shouldPushTerminalSize(lastPushed, rejected)) {
    return { cols: 0, rows: 0 };
  }
  return lastPushed;
}

export function terminalSizeAfterDisabledChange(
  lastPushed: TerminalGridSize,
  wasDisabled: boolean,
  disabled: boolean,
): TerminalGridSize {
  return wasDisabled && !disabled ? { cols: 0, rows: 0 } : lastPushed;
}

export function activationResizeRequest({
  canvasWasDisplayNone,
  wasTransitional,
}: {
  canvasWasDisplayNone: boolean;
  wasTransitional: boolean;
}): { forceResizeDance: boolean; pushBackendSize: boolean } {
  const forceResizeDance = canvasWasDisplayNone && !wasTransitional;
  return {
    forceResizeDance,
    pushBackendSize: !forceResizeDance,
  };
}

const LARGE_ROW_DROP_MIN_ROWS = 6;
const LARGE_ROW_DROP_RATIO = 0.75;

export function isLargeTerminalRowDrop(
  current: TerminalGridSize,
  proposed: TerminalGridSize,
): boolean {
  const rowDrop = current.rows - proposed.rows;
  return (
    rowDrop >= LARGE_ROW_DROP_MIN_ROWS &&
    proposed.rows <= Math.floor(current.rows * LARGE_ROW_DROP_RATIO)
  );
}

export function shouldDelayTerminalResize({
  clearsOnResize,
  current,
  proposed,
  pending,
  allowPending,
}: {
  clearsOnResize: boolean;
  current: TerminalGridSize;
  proposed: TerminalGridSize;
  pending: TerminalGridSize | null;
  allowPending: boolean;
}): boolean {
  if (!clearsOnResize) return false;
  if (!isLargeTerminalRowDrop(current, proposed)) return false;
  if (
    allowPending &&
    pending?.cols === proposed.cols &&
    pending.rows === proposed.rows
  ) {
    return false;
  }
  return true;
}
