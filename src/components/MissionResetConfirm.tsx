// Mission reset confirm dialog. Mirrors Pencil node `DzNZe`:
// destructive-but-recoverable (mission row + crew stay; only the run
// context is wiped). Amber warning border + triangle-alert icon flag
// the destructive intent without dropping into hard-red danger
// territory — that's reserved for archive.

import { useEffect, useRef, useState } from "react";
import { RotateCcw, TriangleAlert } from "lucide-react";

interface MissionResetConfirmProps {
  open: boolean;
  missionTitle: string;
  onConfirm: () => void | Promise<void>;
  onClose: () => void;
}

export function MissionResetConfirm({
  open,
  missionTitle,
  onConfirm,
  onClose,
}: MissionResetConfirmProps) {
  const [submitting, setSubmitting] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    setSubmitting(false);
    // Outside-click + Escape close. Cancel button works the same.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onMouseDown = (e: MouseEvent) => {
      if (!cardRef.current) return;
      if (!cardRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onMouseDown);
    };
  }, [open, onClose]);

  if (!open) return null;

  const handleConfirm = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await onConfirm();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55">
      <div
        ref={cardRef}
        className="flex w-full max-w-[480px] flex-col gap-4 overflow-hidden rounded-xl border-2 border-warn bg-panel shadow-[0_14px_40px_rgba(0,0,0,0.6)]"
      >
        <div className="h-[3px] w-full bg-warn" />
        <div className="flex flex-col gap-4 px-6 pb-6">
          <header className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-warn/15">
              <TriangleAlert aria-hidden className="h-[18px] w-[18px] text-warn" />
            </div>
            <div className="flex min-w-0 flex-col gap-0.5">
              <h2 className="truncate text-[16px] font-semibold text-fg">
                Reset mission?
              </h2>
              <p className="truncate text-[12px] text-fg-3">
                This wipes the run and starts the crew over.
              </p>
            </div>
          </header>
          <div className="flex flex-col gap-1.5 rounded-lg border border-line bg-bg px-3.5 py-3">
            {[
              "All slot PTYs are killed and respawned fresh.",
              "The event log is wiped — feed history is lost.",
              "Agent conversations are dropped — claude-code starts fresh.",
            ].map((line) => (
              <div key={line} className="flex items-start gap-2 text-[12px] leading-snug text-fg-2">
                <span className="select-none text-fg-3">·</span>
                <span>{line}</span>
              </div>
            ))}
          </div>
          <p className="text-[12px] text-fg-3">
            <span className="font-mono text-fg-2">{missionTitle}</span> will
            keep its title, crew, and slots — just nothing else.
          </p>
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="cursor-pointer rounded-md border border-line bg-raised px-3.5 py-2 text-[13px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-default disabled:opacity-60"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void handleConfirm()}
              disabled={submitting}
              className="flex cursor-pointer items-center gap-2 rounded-md bg-warn px-3.5 py-2 text-[13px] font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-default disabled:opacity-60"
            >
              <RotateCcw aria-hidden className="h-3.5 w-3.5" />
              {submitting ? "Resetting…" : "Reset mission"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
