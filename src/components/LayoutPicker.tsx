// Direct-chat layout picker (impl 0020) — TradingView-style preset popup.
//
// Mirrors Pencil node `Stq9b`: presets grouped by pane count (1 · 2 · 3),
// each rendered as a 56×40 mini-diagram tile; the active preset gets an
// accent stroke and accent-tinted panes. Opens from the layout button that
// sits left of Stop in the RunnerChat topbar (`z1hPN` in the mock). The
// popover owns its open state and closes on outside click, Escape, or pick.

import { useEffect, useRef, useState } from "react";

import { SquareSplitHorizontal } from "lucide-react";

import type { PresetKind } from "../lib/paneLayout";

const ROWS: { count: string; kinds: PresetKind[] }[] = [
  { count: "1", kinds: ["single"] },
  { count: "2", kinds: ["cols-2", "rows-2"] },
  { count: "3", kinds: ["main-2", "cols-3", "rows-3"] },
];

const PRESET_LABELS: Record<PresetKind, string> = {
  single: "Single pane",
  "cols-2": "2 side by side",
  "rows-2": "2 stacked",
  "main-2": "1 big + 2 stacked",
  "cols-3": "3 columns",
  "rows-3": "3 rows",
};

export function LayoutPicker({
  active,
  onPick,
}: {
  active: PresetKind;
  onPick: (kind: PresetKind) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        title="Layout"
        aria-label="Layout"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={`inline-flex h-7 w-7 items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg ${
          open ? "bg-raised text-fg" : ""
        }`}
      >
        <SquareSplitHorizontal aria-hidden className="h-[15px] w-[15px]" />
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-50 mt-1.5 flex w-[236px] flex-col gap-3.5 rounded-lg border border-line bg-panel p-3.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
        >
          <span className="font-mono text-[10px] font-semibold uppercase tracking-[0.1em] text-fg-3">
            Layout
          </span>
          {ROWS.map((row) => (
            <div key={row.count} className="flex items-center gap-2.5">
              <span className="w-2 font-mono text-[11px] font-medium text-fg-2">
                {row.count}
              </span>
              <div className="flex items-center gap-2">
                {row.kinds.map((kind) => (
                  <PresetTile
                    key={kind}
                    kind={kind}
                    active={active === kind}
                    onClick={() => {
                      setOpen(false);
                      onPick(kind);
                    }}
                  />
                ))}
              </div>
            </div>
          ))}
          <div className="h-px w-full bg-line" />
          <span className="text-[10px] text-fg-3">
            Layout is remembered across restarts
          </span>
        </div>
      ) : null}
    </div>
  );
}

function PresetTile({
  kind,
  active,
  onClick,
}: {
  kind: PresetKind;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitemradio"
      aria-checked={active}
      title={PRESET_LABELS[kind]}
      aria-label={PRESET_LABELS[kind]}
      onClick={onClick}
      className={`h-10 w-14 cursor-pointer rounded-[5px] border bg-bg p-1 transition-colors ${
        active ? "border-accent" : "border-line-strong hover:border-fg-3"
      }`}
    >
      <PresetDiagram kind={kind} active={active} />
    </button>
  );
}

function PresetDiagram({
  kind,
  active,
}: {
  kind: PresetKind;
  active: boolean;
}) {
  const pane = `min-h-0 min-w-0 flex-1 rounded-[2px] ${
    active ? "bg-accent/15" : "bg-sidebar-selected"
  }`;
  switch (kind) {
    case "single":
      return (
        <span className="flex h-full w-full">
          <span className={pane} />
        </span>
      );
    case "cols-2":
      return (
        <span className="flex h-full w-full gap-[3px]">
          <span className={pane} />
          <span className={pane} />
        </span>
      );
    case "rows-2":
      return (
        <span className="flex h-full w-full flex-col gap-[3px]">
          <span className={pane} />
          <span className={pane} />
        </span>
      );
    case "main-2":
      return (
        <span className="flex h-full w-full gap-[3px]">
          <span className={pane} />
          <span className="flex min-w-0 flex-1 flex-col gap-[3px]">
            <span className={pane} />
            <span className={pane} />
          </span>
        </span>
      );
    case "cols-3":
      return (
        <span className="flex h-full w-full gap-[3px]">
          <span className={pane} />
          <span className={pane} />
          <span className={pane} />
        </span>
      );
    case "rows-3":
      return (
        <span className="flex h-full w-full flex-col gap-[3px]">
          <span className={pane} />
          <span className={pane} />
          <span className={pane} />
        </span>
      );
  }
}
