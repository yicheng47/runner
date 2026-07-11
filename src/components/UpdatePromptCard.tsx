// Arc-style in-app update prompt — impl 0025 (spec `e1jYEa` in
// `design/runner-setting.pen`). Floats in the app sidebar directly
// above the Settings row once an update has downloaded and is ready
// to restart-install. Replaces the old top-center UpdateToast.
//
// Dismissal is per launch (sessionStorage), so the card reappears on
// the next launch until the update is installed. The "Automatic
// updates" checkbox writes the same persisted setting as About's
// toggle — one storage key, two surfaces.

import { useCallback, useState } from "react";
import { Check, X } from "lucide-react";

import { useUpdate } from "../contexts/UpdateContext";
import { STORAGE_AUTO_INSTALL_UPDATES } from "../lib/settings";
import { useStoredBool } from "../lib/useStoredBool";

const DISMISS_KEY = "runner.updatePrompt.dismissed";

function readDismissed(): boolean {
  try {
    return sessionStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

export function UpdatePromptCard() {
  const { status, restart } = useUpdate();
  const [dismissed, setDismissed] = useState<boolean>(() => readDismissed());
  const [autoInstall, setAutoInstall] = useStoredBool(
    STORAGE_AUTO_INSTALL_UPDATES,
    true,
  );

  const dismiss = useCallback(() => {
    setDismissed(true);
    try {
      sessionStorage.setItem(DISMISS_KEY, "1");
    } catch {
      // best-effort — in-memory dismissal still holds for this window.
    }
  }, []);

  if (status !== "ready" || dismissed) return null;

  return (
    <div className="mx-3 mb-2 overflow-hidden rounded-lg border border-line-strong bg-[radial-gradient(140%_120%_at_50%_0%,var(--color-sidebar-selected)_0%,var(--color-panel)_100%)] shadow-[0_8px_24px_rgba(0,0,0,0.35)]">
      {/* Full-bleed header band with the dismiss affordance. */}
      <div className="flex items-center justify-between gap-2 border-b border-line-strong bg-sidebar-selected px-3 py-2">
        <span className="min-w-0 truncate text-[12px] font-medium text-fg">
          New Runner version available
        </span>
        <button
          type="button"
          onClick={dismiss}
          aria-label="Dismiss update prompt"
          className="flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg"
        >
          <X aria-hidden className="h-3 w-3" />
        </button>
      </div>
      <div className="flex flex-col gap-2.5 px-3 py-2.5">
        <button
          type="button"
          role="checkbox"
          aria-checked={autoInstall}
          onClick={() => setAutoInstall(!autoInstall)}
          className="flex cursor-pointer items-center gap-2 text-left text-[11px] text-fg-2 transition-colors hover:text-fg"
        >
          <span
            className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors ${
              autoInstall
                ? "border-accent bg-accent text-accent-ink"
                : "border-line-strong bg-raised"
            }`}
          >
            {autoInstall ? <Check aria-hidden className="h-2.5 w-2.5" /> : null}
          </span>
          Automatic updates
        </button>
        <button
          type="button"
          onClick={() => void restart()}
          className="relative h-[26px] w-full cursor-pointer overflow-hidden rounded-md bg-accent text-[12px] font-semibold text-accent-ink transition-opacity hover:opacity-90"
        >
          {/* Center-glow highlight over the accent fill. */}
          <span
            aria-hidden
            className="pointer-events-none absolute inset-0 bg-[radial-gradient(60%_140%_at_50%_50%,rgba(255,255,255,0.28),rgba(255,255,255,0)_70%)]"
          />
          <span className="relative">Restart and Update</span>
        </button>
      </div>
    </div>
  );
}
