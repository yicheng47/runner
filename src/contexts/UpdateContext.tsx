// Auto-update context — single source of update state shared by the
// sidebar prompt card (`UpdatePromptCard`) and the Settings → About
// pane. Mirrors Quill's pattern so card → settings is just two views
// over the same status, no duplicated polling.

import {
  createContext,
  useContext,
  useEffect,
  useRef,
  type ReactNode,
} from "react";

import {
  readStoredBool,
  STORAGE_AUTO_INSTALL_UPDATES,
} from "../lib/settings";
import { useUpdateChecker, type UpdateState } from "../hooks/useUpdateChecker";

const UpdateContext = createContext<UpdateState | null>(null);

export function UpdateProvider({ children }: { children: ReactNode }) {
  const state = useUpdateChecker();
  // The auto-install toggle owns two separate behaviors: (1) whether
  // we even kick off a check on app start, and (2) whether we go
  // straight to download once an update is detected. Persistent
  // store is plain localStorage for now (same key the About pane
  // toggle and the prompt card checkbox write to).
  const checkedRef = useRef(false);

  // Run a single check ~3s after mount. The delay keeps the launch
  // path quiet — first paint, navigation, sidebar load all happen
  // before we hit the network.
  useEffect(() => {
    if (checkedRef.current) return;
    checkedRef.current = true;
    if (!readStoredBool(STORAGE_AUTO_INSTALL_UPDATES, true)) return;
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
  // "downloading" automatically. Otherwise the user has to click
  // Download in Settings → About.
  useEffect(() => {
    if (state.status !== "available") return;
    if (!readStoredBool(STORAGE_AUTO_INSTALL_UPDATES, true)) return;
    void state.downloadAndInstall();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.status]);

  return (
    <UpdateContext.Provider value={state}>{children}</UpdateContext.Provider>
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
