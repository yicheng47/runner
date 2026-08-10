const HUMAN_SEED = "human";
const HUMAN_HUE = "var(--color-warn)";
const RUNNER_HUES = [
  "#ff8a4c",
  "#c792ea",
  "#f07178",
  "var(--color-accent)",
  "#62d9f5",
  "#a3e635",
  "#7aa2f7",
  "#ff6b9d",
  "#c3e88d",
] as const;

const HUMAN_SOURCE_CELLS = [
  true,
  false,
  true,
  false,
  true,
  true,
  true,
  true,
  false,
  false,
  true,
  true,
  true,
  false,
  true,
] as const;

export type RunnerPresence = "busy" | "idle" | "stopped" | "crashed";

interface RunnerAvatarProps {
  seed: string;
  size: number;
  presence?: RunnerPresence;
}

function fnv1a(seed: string): number {
  let hash = 0x811c9dc5;
  for (const byte of new TextEncoder().encode(seed)) {
    hash ^= byte;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

// eslint-disable-next-line react-refresh/only-export-components
export const hueForSeed = (seed: string): string => {
  if (seed === HUMAN_SEED) return HUMAN_HUE;
  const hash = fnv1a(seed);
  return RUNNER_HUES[(hash >>> 16) % RUNNER_HUES.length];
};

function cellsForSeed(seed: string): boolean[] {
  const hash = fnv1a(seed);
  const source: boolean[] =
    seed === HUMAN_SEED
      ? [...HUMAN_SOURCE_CELLS]
      : Array.from({ length: 15 }, (_, bit) => Boolean(hash & (1 << bit)));
  if (!source.some(Boolean)) source[8] = true;

  return Array.from({ length: 25 }, (_, index) => {
    const row = Math.floor(index / 5);
    const column = index % 5;
    const sourceColumn = column < 3 ? column : 4 - column;
    return source[row * 3 + sourceColumn];
  });
}

const PRESENCE_CLASS: Record<RunnerPresence, string> = {
  busy: "bg-accent",
  idle: "bg-accent/40",
  stopped: "bg-fg-3",
  crashed: "bg-danger",
};

export function RunnerAvatar({ seed, size, presence }: RunnerAvatarProps) {
  const hue = hueForSeed(seed);
  const cells = cellsForSeed(seed);

  return (
    <span
      aria-hidden
      className="relative inline-flex shrink-0"
      style={{ width: size, height: size }}
    >
      <svg
        viewBox="0 0 5 5"
        className="h-full w-full overflow-hidden bg-raised"
        style={{ borderRadius: Math.max(4, Math.round(size * 0.23)) }}
        shapeRendering="crispEdges"
      >
        {cells.map((painted, index) =>
          painted ? (
            <rect
              key={index}
              x={index % 5}
              y={Math.floor(index / 5)}
              width="1"
              height="1"
              fill={hue}
            />
          ) : null,
        )}
      </svg>
      {presence ? (
        <span
          className={`absolute -bottom-px -right-px h-[9px] w-[9px] rounded-full border-2 border-bg ${PRESENCE_CLASS[presence]}`}
        />
      ) : null}
    </span>
  );
}
