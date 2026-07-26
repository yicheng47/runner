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
