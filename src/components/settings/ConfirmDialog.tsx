// Destructive confirm dialog — mirrors Pencil `component/confirm-dialog`
// in design/runner-setting.pen. One reusable surface for the Archived
// pane's delete flows (Delete all today, per-row trash later) so the
// styling stays synced. Esc and outside-click cancel, matching
// MissionResetConfirm; both are ignored while `busy` so a mid-flight
// deletion can't be left visually orphaned.

import { useEffect, useRef } from "react";
import { Trash2 } from "lucide-react";

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  busyLabel,
  busy,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body: string;
  confirmLabel: string;
  busyLabel: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || busy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    const onMouseDown = (e: MouseEvent) => {
      if (cardRef.current && !cardRef.current.contains(e.target as Node)) {
        onCancel();
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onMouseDown);
    };
  }, [open, busy, onCancel]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55">
      <div
        ref={cardRef}
        className="flex w-[420px] flex-col gap-3.5 rounded-xl border border-line bg-panel px-[22px] py-5 shadow-[0_16px_48px_rgba(0,0,0,0.4)]"
      >
        <header className="flex items-center gap-2.5">
          <Trash2 aria-hidden className="h-[15px] w-[15px] shrink-0 text-danger" />
          <h2 className="text-[15px] font-semibold text-fg">{title}</h2>
        </header>
        <p className="text-[13px] leading-relaxed text-fg-2">{body}</p>
        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="cursor-pointer rounded-lg bg-raised px-3.5 py-1.5 text-[12px] font-medium text-fg transition-colors hover:bg-raised/80 disabled:cursor-default disabled:opacity-60"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            disabled={busy}
            className="cursor-pointer rounded-lg border border-danger/40 bg-danger/10 px-3.5 py-1.5 text-[12px] font-medium text-danger transition-colors hover:bg-danger/20 disabled:cursor-default disabled:opacity-60"
          >
            {busy ? busyLabel : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
