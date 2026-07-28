export interface TerminalReplayChunk {
  data: Uint8Array;
  width?: number | null;
}

interface ReplayTerminal {
  resize(cols: number, rows: number): void;
  write(data: Uint8Array, callback: () => void): void;
}

interface ReplayOptions {
  terminal: ReplayTerminal;
  initialChunks: TerminalReplayChunk[];
  takePendingChunks: () => TerminalReplayChunk[];
  fallbackWidth: number;
  target: { cols: number; rows: number };
  setResizeSuppressed: (suppressed: boolean) => void;
  shouldContinue: () => boolean;
}

function chunkWidth(chunk: TerminalReplayChunk, fallbackWidth: number): number {
  return Number.isInteger(chunk.width) && (chunk.width ?? 0) > 0
    ? (chunk.width as number)
    : fallbackWidth;
}

function joinChunks(
  chunks: TerminalReplayChunk[],
  start: number,
  end: number,
): Uint8Array {
  if (end - start === 1) return chunks[start].data;
  let length = 0;
  for (let index = start; index < end; index += 1) {
    length += chunks[index].data.length;
  }
  const joined = new Uint8Array(length);
  let offset = 0;
  for (let index = start; index < end; index += 1) {
    joined.set(chunks[index].data, offset);
    offset += chunks[index].data.length;
  }
  return joined;
}

function write(
  terminal: ReplayTerminal,
  data: Uint8Array,
): Promise<void> {
  return new Promise((resolve) => terminal.write(data, resolve));
}

export async function replayOutputAtWidths({
  terminal,
  initialChunks,
  takePendingChunks,
  fallbackWidth,
  target,
  setResizeSuppressed,
  shouldContinue,
}: ReplayOptions): Promise<boolean> {
  setResizeSuppressed(true);
  try {
    let chunks = initialChunks;
    while (chunks.length > 0) {
      let start = 0;
      while (start < chunks.length) {
        if (!shouldContinue()) return false;
        const width = chunkWidth(chunks[start], fallbackWidth);
        let end = start + 1;
        while (
          end < chunks.length &&
          chunkWidth(chunks[end], fallbackWidth) === width
        ) {
          end += 1;
        }
        terminal.resize(width, target.rows);
        await write(terminal, joinChunks(chunks, start, end));
        start = end;
      }
      chunks = takePendingChunks();
    }
    if (!shouldContinue()) return false;
    terminal.resize(target.cols, target.rows);
    return true;
  } finally {
    setResizeSuppressed(false);
  }
}
