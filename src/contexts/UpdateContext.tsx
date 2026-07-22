// Auto-update context — single source of update state shared by the
// sidebar prompt card (`UpdatePromptCard`) and the Settings → Updates
// pane. Mirrors Quill's pattern so card → settings is just two views
// over the same status, no duplicated polling.

import {
  useCallback,
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";

import { readStoredBool, STORAGE_AUTO_INSTALL_UPDATES } from "../lib/settings";
import {
  useUpdateChecker,
  type UpdateCheckOptions,
  type UpdateState,
} from "../hooks/useUpdateChecker";

const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const LAUNCH_CHECK_DELAY_MS = 3000;

const UpdateContext = createContext<UpdateState | null>(null);

export function UpdateProvider({ children }: { children: ReactNode }) {
  const state = useUpdateChecker();
  const [runsBackgroundChecks] = useState(() => {
    try {
      return getCurrentWindow().label === "main";
    } catch {
      return true;
    }
  });
  const runCheck = state.checkForUpdate;
  const lastCheckAt = useRef<number | null>(null);
  const checkForUpdate = useCallback(
    (options?: UpdateCheckOptions) => {
      lastCheckAt.current = Date.now();
      return runCheck(options);
    },
    [runCheck],
  );

  // Check after first paint on every launch, unless an explicit check
  // already ran during the delay.
  useEffect(() => {
    if (!runsBackgroundChecks) return;
    const timer = setTimeout(() => {
      if (lastCheckAt.current === null) {
        void checkForUpdate({ silent: true });
      }
    }, LAUNCH_CHECK_DELAY_MS);
    return () => clearTimeout(timer);
  }, [checkForUpdate, runsBackgroundChecks]);

  useEffect(() => {
    if (!runsBackgroundChecks) return;
    const timer = setInterval(() => {
      void checkForUpdate({ silent: true });
    }, UPDATE_CHECK_INTERVAL_MS);
    const checkWhenStale = () => {
      const checkedAt = lastCheckAt.current;
      if (
        checkedAt !== null &&
        Date.now() - checkedAt >= UPDATE_CHECK_INTERVAL_MS
      ) {
        void checkForUpdate({ silent: true });
      }
    };
    window.addEventListener("focus", checkWhenStale);
    let unlistenFocus: (() => void) | null = null;
    let cancelled = false;
    try {
      void getCurrentWindow()
        .onFocusChanged(({ payload: focused }) => {
          if (focused) checkWhenStale();
        })
        .then((stop) => {
          if (cancelled) {
            stop();
            return;
          }
          unlistenFocus = stop;
        })
        .catch(() => {
          // DOM focus remains available in browser preview.
        });
    } catch {
      // DOM focus remains available in browser preview.
    }
    return () => {
      cancelled = true;
      unlistenFocus?.();
      clearInterval(timer);
      window.removeEventListener("focus", checkWhenStale);
    };
  }, [checkForUpdate, runsBackgroundChecks]);

  // When the user has auto-install on, advance from "available" →
  // "downloading" automatically. Otherwise the user has to click
  // Download in Settings → Updates.
  useEffect(() => {
    if (state.status !== "available") return;
    if (!readStoredBool(STORAGE_AUTO_INSTALL_UPDATES, true)) return;
    void state.downloadAndInstall();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.status]);

  return (
    <UpdateContext.Provider value={{ ...state, checkForUpdate }}>
      {children}
    </UpdateContext.Provider>
  );
}

// Co-located with the provider so a single import gives consumers
// the full context surface. The Fast-Refresh rule wants components
// and non-components in separate files; splitting them here would
// just create a one-liner module for the hook, so disable instead.
// eslint-disable-next-line react-refresh/only-export-components
export function useUpdate(): UpdateState {
  const ctx = useContext(UpdateContext);
  if (!ctx) throw new Error("useUpdate must be used within UpdateProvider");
  return ctx;
}
