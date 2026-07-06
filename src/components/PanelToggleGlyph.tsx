// Two-state panel-toggle glyph shared by the left sidebar (AppShell) and
// the right runner rail (RunnerChat / MissionWorkspace), so both toggles
// read as one family. Drawn to the #246 Pencil spec (node n96Xi): a
// landscape 60×46 panel — 3px outline, 9px radius, with a filled column /
// divider one third in from the panel's docked edge.
//   filled → solid column on the docked edge = panel SHOWN
//   hollow → thin divider, no fill            = panel HIDDEN
// `side` mirrors the geometry: "left" hugs the leading edge (sidebar),
// "right" hugs the trailing edge (runner rail). lucide's PanelLeft/Right
// are square and use a solid-vs-dashed divider instead, so both states
// are hand-rolled here to keep the design's aspect ratio and fill.
// The viewBox carries a 2-unit margin (`-2 -2 64 50`) so the outer stroke
// isn't flush with the boundary — otherwise it clips/softens when scaled
// down small.
export function PanelToggleGlyph({
  side,
  filled,
  className,
}: {
  side: "left" | "right";
  filled: boolean;
  className?: string;
}) {
  return (
    <svg viewBox="-2 -2 64 50" fill="none" aria-hidden className={className}>
      <rect
        x="1.5"
        y="1.5"
        width="57"
        height="43"
        rx="7.5"
        stroke="currentColor"
        strokeWidth={3}
      />
      {filled ? (
        <path
          d={
            side === "left"
              ? "M9 3 H19 V43 H9 Q3 43 3 37 V9 Q3 3 9 3 Z"
              : "M51 3 H41 V43 H51 Q57 43 57 37 V9 Q57 3 51 3 Z"
          }
          fill="currentColor"
        />
      ) : (
        <rect
          x={side === "left" ? 19 : 38}
          y="3"
          width="3"
          height="40"
          rx="1.5"
          fill="currentColor"
        />
      )}
    </svg>
  );
}
