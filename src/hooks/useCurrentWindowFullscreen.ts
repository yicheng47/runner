import { useEffect, useState } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";

export const FULLSCREEN_SETTLE_MS = 200;

export function useCurrentWindowFullscreen(): boolean {
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    let currentWindow: ReturnType<typeof getCurrentWindow>;
    try {
      currentWindow = getCurrentWindow();
    } catch {
      return;
    }
    if (
      typeof currentWindow.isFullscreen !== "function" ||
      typeof currentWindow.onResized !== "function"
    ) {
      return;
    }

    let active = true;
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    let unlisten: (() => void) | null = null;

    const refresh = () => {
      try {
        void currentWindow
          .isFullscreen()
          .then((next) => {
            if (active) setFullscreen(next);
          })
          .catch(() => {});
      } catch {
        return;
      }
    };

    refresh();
    try {
      void currentWindow
        .onResized(() => {
          if (settleTimer) clearTimeout(settleTimer);
          settleTimer = setTimeout(refresh, FULLSCREEN_SETTLE_MS);
        })
        .then((stopListening) => {
          if (active) unlisten = stopListening;
          else stopListening();
        })
        .catch(() => {});
    } catch {
      // Browser preview without the Tauri event runtime.
    }

    return () => {
      active = false;
      if (settleTimer) clearTimeout(settleTimer);
      unlisten?.();
    };
  }, []);

  return fullscreen;
}
