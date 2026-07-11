// Arc-style in-app update prompt, current-Arc form: at rest it's a
// slim centered pill ("New Runner version available") directly above
// the sidebar Settings row; hovering (or keyboard-focusing) it morphs
// the pill into the full card — the card is bottom-anchored OVER the
// pill and grows upward, so its header band reads as the pill risen
// to the top and the two are never visible at once. Supersedes the
// always-expanded card from spec `e1jYEa` per user direction.
//
// Dismissal is per launch (sessionStorage), so the pill reappears on
// the next launch until the update is installed. The checkbox writes
// the same persisted setting as About's toggle — one storage key,
// two surfaces.

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
    <div className="group relative mx-3 mb-2">
      {/* Resting pill — the only thing visible until hover. A real
          button so it's natively focusable; the explicit focus() on
          click covers WebKit, which doesn't focus buttons on click,
          so a click also pins the card open via focus-within. It
          renders BEFORE the card so Tab walks trigger → revealed
          controls in order. */}
      <button
        type="button"
        aria-haspopup="true"
        onClick={(e) => e.currentTarget.focus()}
        className="flex h-6 w-full cursor-pointer items-center justify-center rounded-full border border-sidebar-selected-border bg-sidebar-selected/60 px-3 focus:outline-none"
      >
        <span className="truncate whitespace-nowrap text-[11px] font-medium text-fg-2">
          New Runner version available
        </span>
      </button>
      {/* Pop-up card — bottom edge pinned to the pill's bottom
          (`bottom-0`), so it covers the pill and grows upward over
          the list above. No gap to hover across, and the pill is
          never visible alongside the card. */}
      <div className="invisible absolute bottom-0 left-0 right-0 z-40 translate-y-1 opacity-0 transition-all duration-150 group-focus-within:visible group-focus-within:translate-y-0 group-focus-within:opacity-100 group-hover:visible group-hover:translate-y-0 group-hover:opacity-100">
        <div className="overflow-hidden rounded-lg border border-line-strong bg-[radial-gradient(140%_120%_at_50%_0%,var(--color-sidebar-selected)_0%,var(--color-panel)_100%)] shadow-[0_8px_24px_rgba(0,0,0,0.35)]">
          {/* Header band — title centered per spec; the × dismiss
              (now specced) stays pinned to the band's right, with px-6
              reserving symmetric space so the title centers truly. */}
          <div className="relative flex items-center justify-center border-b border-line-strong bg-sidebar-selected px-6 py-1.5">
            <span className="min-w-0 truncate text-[11px] font-medium text-fg">
              New Runner version available
            </span>
            <button
              type="button"
              onClick={dismiss}
              aria-label="Dismiss update prompt"
              className="absolute right-2 top-1/2 flex h-4 w-4 -translate-y-1/2 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg"
            >
              <X aria-hidden className="h-2.5 w-2.5" />
            </button>
          </div>
          <div className="flex flex-col gap-2 px-3 py-2">
            {/* Checked state is deliberately NEUTRAL (spec hexes) —
                per the settings design rule, accent green marks only
                the meaning moment: the Restart button below. */}
            <button
              type="button"
              role="checkbox"
              aria-checked={autoInstall}
              onClick={() => setAutoInstall(!autoInstall)}
              className="flex cursor-pointer items-center gap-2 self-center text-left text-[11px] text-fg-2 transition-colors hover:text-fg"
            >
              <span
                className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors ${
                  autoInstall
                    ? "border-[#4A4C56] bg-[#3A3C46] text-fg"
                    : "border-line-strong bg-raised"
                }`}
              >
                {autoInstall ? (
                  <Check aria-hidden className="h-2.5 w-2.5" />
                ) : null}
              </span>
              Automatic updates
            </button>
            <button
              type="button"
              onClick={() => void restart()}
              className="relative h-[26px] w-full cursor-pointer overflow-hidden rounded-md bg-accent text-[12px] font-semibold text-accent-ink transition-opacity hover:opacity-90"
            >
              <span
                aria-hidden
                className="pointer-events-none absolute inset-0 bg-[radial-gradient(60%_140%_at_50%_50%,rgba(255,255,255,0.28),rgba(255,255,255,0)_70%)]"
              />
              <span className="relative">Restart and Update</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
