// Update toast — top-center pill that stays visible through the full
// auto-update lifecycle (available → downloading → ready). Copy and
// action swap with `status`; the outer shell stays identical so the
// user sees one persistent surface. Mirrors design `To8GR` and
// Quill's UpdateToast; progress bar matches Settings → Updates.

import { useCallback, useEffect, useState } from "react";

import { X } from "lucide-react";

import { useUpdate } from "../contexts/UpdateContext";
import {
  readStoredBool,
  STORAGE_AUTO_INSTALL_UPDATES,
} from "../lib/settings";

interface UpdateToastProps {
  onOpenSettings: () => void;
}

export function UpdateToast({ onOpenSettings }: UpdateToastProps) {
  const { status, update, progress, restart } = useUpdate();
  const [dismissed, setDismissed] = useState(false);
  const [visible, setVisible] = useState(false);

  const shouldShow =
    (status === "available" ||
      status === "downloading" ||
      status === "ready") &&
    !dismissed;

  useEffect(() => {
    if (!shouldShow) {
      setVisible(false);
      return;
    }
    // requestAnimationFrame triggers the enter transition once
    // mounted — without it the element would slide in from a stale
    // computed style.
    const raf = requestAnimationFrame(() => setVisible(true));
    // 30s auto-dismiss applies only to the manual-install "available"
    // prompt. For downloading/ready we keep the toast up until the
    // user dismisses or restarts.
    const armTimer =
      status === "available" &&
      !readStoredBool(STORAGE_AUTO_INSTALL_UPDATES, true);
    const timer = armTimer
      ? setTimeout(() => {
          setVisible(false);
          setTimeout(() => setDismissed(true), 200);
        }, 30000)
      : null;
    return () => {
      cancelAnimationFrame(raf);
      if (timer) clearTimeout(timer);
    };
  }, [shouldShow, status]);

  const dismiss = useCallback(() => {
    setVisible(false);
    setTimeout(() => setDismissed(true), 200);
  }, []);

  const handleUpdate = useCallback(() => {
    dismiss();
    onOpenSettings();
  }, [dismiss, onOpenSettings]);

  const handleRestart = useCallback(() => {
    void restart();
  }, [restart]);

  if (!shouldShow) return null;

  const version = update?.version ?? "";

  const label =
    status === "downloading"
      ? `Downloading Runner v${version}… ${progress}%`
      : status === "ready"
        ? `Runner v${version} is ready to install`
        : `Runner v${version} is available`;

  return (
    <div
      className={`fixed left-1/2 top-5 z-50 flex -translate-x-1/2 flex-col gap-2 rounded-[14px] border border-line-strong bg-panel py-2.5 pl-4 pr-3 shadow-[0_8px_24px_rgba(0,0,0,0.5)] transition-all duration-200 ${
        visible ? "translate-y-0 opacity-100" : "-translate-y-2 opacity-0"
      }`}
      role="status"
    >
      <div className="flex items-center gap-3">
        <span className="whitespace-nowrap text-[13px] tracking-[-0.08px] text-fg">
          {label}
        </span>
        {status === "available" ? (
          <button
            type="button"
            onClick={handleUpdate}
            className="cursor-pointer whitespace-nowrap rounded-md bg-accent/15 px-2.5 py-1 text-[13px] font-medium text-accent transition-colors hover:bg-accent/25"
          >
            Update
          </button>
        ) : null}
        {status === "ready" ? (
          <button
            type="button"
            onClick={handleRestart}
            className="cursor-pointer whitespace-nowrap rounded-md bg-accent/15 px-2.5 py-1 text-[13px] font-medium text-accent transition-colors hover:bg-accent/25"
          >
            Restart
          </button>
        ) : null}
        <button
          type="button"
          onClick={dismiss}
          aria-label="Dismiss update notification"
          className="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-raised hover:text-fg"
        >
          <X size={14} aria-hidden />
        </button>
      </div>
      {status === "downloading" ? (
        <div className="h-[3px] w-full overflow-hidden rounded-full bg-raised">
          <div
            className="h-full rounded-full bg-accent transition-[width] duration-200"
            style={{ width: `${progress}%` }}
          />
        </div>
      ) : null}
    </div>
  );
}
