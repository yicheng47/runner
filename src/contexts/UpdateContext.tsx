// Auto-update context — single source of update state shared by the
// shell-level toast (`UpdateToast`) and the Settings → Updates pane.
// Mirrors Quill's pattern so toast → settings is just two views over
// the same status, no duplicated polling.

import {
  createContext,
  useContext,
  useEffect,
  useRef,
  type ReactNode,
} from "react";

import { useUpdateChecker, type UpdateState } from "../hooks/useUpdateChecker";

const UpdateContext = createContext<UpdateState | null>(null);

const AUTO_INSTALL_KEY = "settings.autoInstallUpdates";

export function UpdateProvider({ children }: { children: ReactNode }) {
  const state = useUpdateChecker();
  // The auto-install toggle owns two separate behaviors: (1) whether
  // we even kick off a check on app start, and (2) whether we go
  // straight to download once an update is detected. Persistent
  // store is plain localStorage for now (same key the Updates pane
  // toggle writes to).
  const checkedRef = useRef(false);

  // Run a single check ~3s after mount. The delay keeps the launch
  // path quiet — first paint, navigation, sidebar load all happen
  // before we hit the network.
  useEffect(() => {
    if (checkedRef.current) return;
    checkedRef.current = true;
    const enabled = localStorage.getItem(AUTO_INSTALL_KEY) !== "false";
    if (!enabled) return;
    const timer = setTimeout(() => {
      void state.checkForUpdate();
    }, 3000);
    return () => clearTimeout(timer);
    // checkForUpdate is stable (useCallback with []), so it's safe
    // to omit from deps; running this exactly once per mount is the
    // intent.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // When the user has auto-install on, advance from "available" →
  // "downloading" automatically. The toast still shows the available
  // pill briefly so it doesn't feel like the update happened in
  // secret. Otherwise the user has to click Download in Settings.
  useEffect(() => {
    if (state.status !== "available") return;
    const enabled = localStorage.getItem(AUTO_INSTALL_KEY) !== "false";
    if (!enabled) return;
    void state.downloadAndInstall();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.status]);

  return (
    <UpdateContext.Provider value={state}>{children}</UpdateContext.Provider>
  );
}

export function useUpdate(): UpdateState {
  const ctx = useContext(UpdateContext);
  if (!ctx) throw new Error("useUpdate must be used within UpdateProvider");
  return ctx;
}
