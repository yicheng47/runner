export interface TerminalGridSize {
  cols: number;
  rows: number;
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
