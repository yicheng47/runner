// Update-available toast — top-center pill that nudges the user
// toward Settings → Updates when a new release is detected. The
// click handler does NOT download; it only opens the settings pane.
// The download flow lives there so users can read release notes
// (eventually) and confirm. Mirrors design `To8GR` and Quill's
// UpdateToast.

import { useCallback, useEffect, useState } from "react";

import { X } from "lucide-react";

import { useUpdate } from "../contexts/UpdateContext";

interface UpdateToastProps {
  onOpenSettings: () => void;
}

export function UpdateToast({ onOpenSettings }: UpdateToastProps) {
  const { status, update } = useUpdate();
  const [dismissed, setDismissed] = useState(false);
  const [visible, setVisible] = useState(false);

  // We only show the toast for the "available" state. Once the user
  // is in Settings the pane handles "downloading" / "ready" itself,
  // and surfacing those in a toast too would be noisy.
  const shouldShow = status === "available" && !dismissed;

  useEffect(() => {
    if (!shouldShow) {
      setVisible(false);
      return;
    }
    // requestAnimationFrame triggers the enter transition once
    // mounted — without it the element would slide in from a stale
    // computed style.
    const raf = requestAnimationFrame(() => setVisible(true));
    // Auto-dismiss after 30s so a missed update doesn't hang around
    // forever. The same toast comes back next launch if still
    // available.
    const timer = setTimeout(() => {
      setVisible(false);
      setTimeout(() => setDismissed(true), 200);
    }, 30000);
    return () => {
      cancelAnimationFrame(raf);
      clearTimeout(timer);
    };
  }, [shouldShow]);

  const dismiss = useCallback(() => {
    setVisible(false);
    setTimeout(() => setDismissed(true), 200);
  }, []);

  const handleUpdate = useCallback(() => {
    dismiss();
    onOpenSettings();
  }, [dismiss, onOpenSettings]);

  if (!shouldShow) return null;

  const version = update?.version ?? "";

  return (
    <div
      className={`fixed left-1/2 top-5 z-50 flex -translate-x-1/2 items-center gap-3 rounded-[14px] border border-line-strong bg-panel py-2.5 pl-4 pr-3 shadow-[0_8px_24px_rgba(0,0,0,0.5)] transition-all duration-200 ${
        visible ? "translate-y-0 opacity-100" : "-translate-y-2 opacity-0"
      }`}
      role="status"
    >
      <span className="text-[13px] tracking-[-0.08px] text-fg whitespace-nowrap">
        Runner v{version} is available
      </span>
      <button
        type="button"
        onClick={handleUpdate}
        className="cursor-pointer whitespace-nowrap rounded-md bg-accent/15 px-2.5 py-1 text-[13px] font-medium text-accent transition-colors hover:bg-accent/25"
      >
        Update
      </button>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss update notification"
        className="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-raised hover:text-fg"
      >
        <X size={14} aria-hidden />
      </button>
    </div>
  );
}
