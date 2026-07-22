import { useEffect, useRef } from "react";
import {
  BrowserRouter,
  Routes,
  Route,
  Navigate,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AppShell } from "./components/AppShell";
import { ToastProvider } from "./contexts/ToastContext";
import { UpdateProvider, useUpdate } from "./contexts/UpdateContext";
import { nudgeAppZoom } from "./lib/appZoom";
import { eventMatchesShortcut } from "./lib/keymap";
import { readAppZoom } from "./lib/settings";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";
import Runners from "./pages/Runners";
import RunnerDetail from "./pages/RunnerDetail";

export default function App() {
  // Tell the backend the first frame has painted so it can show + focus the
  // main window. Don't wrap this in requestAnimationFrame — macOS pauses rAF
  // for hidden windows, so the rAF callback would never fire and the window
  // would stay hidden forever. useEffect runs after React commits, which is
  // enough to guarantee a non-blank webview before show.
  useEffect(() => {
    invoke("app_ready").catch(console.error);
  }, []);

  // Restore the user's persisted app zoom on boot. Skipped when the stored
  // value is the default (1.0) so we don't roundtrip through Tauri for a
  // no-op. Wrapped in try/catch because dev browser preview has no Tauri
  // webview API.
  useEffect(() => {
    const zoom = readAppZoom();
    if (zoom === 1.0) return;
    try {
      void getCurrentWebview().setZoom(zoom).catch(() => {
        // best-effort — webview swap or platform refusal shouldn't block boot.
      });
    } catch {
      // No Tauri runtime (dev browser preview).
    }
  }, []);

  // Zoom shortcuts (keymap: zoom-in / zoom-out / zoom-reset). Capture
  // phase so xterm's textarea doesn't swallow the key before us;
  // preventDefault only on matches so other Cmd-key combos (copy,
  // paste, etc.) still work.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = eventMatchesShortcut(e, "zoom-in")
        ? (1 as const)
        : eventMatchesShortcut(e, "zoom-out")
          ? (-1 as const)
          : eventMatchesShortcut(e, "zoom-reset")
            ? ("reset" as const)
            : null;
      if (action === null) return;
      e.preventDefault();
      e.stopPropagation();
      nudgeAppZoom(action);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <UpdateProvider>
      <ToastProvider>
        <BrowserRouter>
          <InitialRouteBootstrap />
          <SettingsShortcut />
          <UpdateMenuListener />
          <Routes>
            <Route element={<AppShell />}>
              <Route path="/" element={<Navigate to="/runners" replace />} />
              <Route path="/crews" element={<Crews />} />
              <Route path="/crews/:crewId" element={<CrewEditor />} />
              <Route path="/runners" element={<Runners />} />
              <Route path="/runners/:handle" element={<RunnerDetail />} />
              {/* Null elements on purpose: the chat surface and mission
                  workspace render through AppShell's PersistentSurfaces
                  layer, which keeps them mounted across route changes so
                  terminals survive visits to the list pages. The routes
                  still exist so matching works and `*` doesn't redirect. */}
              <Route path="/chats/:sessionId" element={null} />
              <Route path="/missions/:id" element={null} />
              {/* Settings takes over the whole window (impl 0025) but
                  renders through AppShell's takeover layer, not the
                  Outlet: routing it outside AppShell unmounted the
                  shell — and with it PersistentSurfaces — so every
                  Settings round-trip cold-remounted the terminals
                  (the half-width-repaint path #309 closed for list
                  pages). */}
              <Route path="/settings/:pane?" element={null} />
              <Route path="*" element={<Navigate to="/runners" replace />} />
            </Route>
          </Routes>
        </BrowserRouter>
      </ToastProvider>
    </UpdateProvider>
  );
}

// Settings shortcut (keymap: open-settings) — wired app-side, not an
// OS menu accelerator. Like a native menu item it fires even while a
// text field is focused; it only navigates, so a stray hit is
// harmless. Two entry points, mirroring the toggle-sidebar pattern:
// the window capture listener for ordinary keystrokes, plus
// RunnerTerminal's re-dispatched custom event for keys WKWebView
// delivers straight to xterm.
const OPEN_SETTINGS_EVENT = "runner:open-settings";
const CHECK_FOR_UPDATES_EVENT = "runner/check-for-updates";

function SettingsShortcut() {
  const navigate = useNavigate();
  const location = useLocation();
  useEffect(() => {
    const openSettings = () => {
      if (!location.pathname.startsWith("/settings")) {
        navigate("/settings", { state: { from: location.pathname } });
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (!eventMatchesShortcut(e, "open-settings")) return;
      e.preventDefault();
      e.stopPropagation();
      openSettings();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener(OPEN_SETTINGS_EVENT, openSettings);
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener(OPEN_SETTINGS_EVENT, openSettings);
    };
  }, [navigate, location.pathname]);
  return null;
}

function UpdateMenuListener() {
  const navigate = useNavigate();
  const location = useLocation();
  const locationRef = useRef(location);
  const { checkForUpdate } = useUpdate();

  useEffect(() => {
    locationRef.current = location;
  }, [location]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    let target: { target: { kind: "WebviewWindow"; label: string } } | undefined;
    try {
      target = {
        target: { kind: "WebviewWindow", label: getCurrentWebview().label },
      };
    } catch {
      target = undefined;
    }
    void listen(CHECK_FOR_UPDATES_EVENT, () => {
      const current = locationRef.current;
      const from = current.pathname.startsWith("/settings")
        ? ((current.state as { from?: string } | null)?.from ?? "/")
        : current.pathname;
      void checkForUpdate();
      if (current.pathname !== "/settings/updates") {
        navigate("/settings/updates", { state: { from } });
      }
    }, target)
      .then((stop) => {
        if (cancelled) {
          stop();
          return;
        }
        unlisten = stop;
      })
      .catch(() => {
        // Browser preview has no Tauri event runtime.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [checkForUpdate, navigate]);
  return null;
}

// Secondary windows are opened at `index.html#/missions/<id>` because
// BrowserRouter can't resolve a deep path through Tauri's asset protocol in
// release builds (impl 0018 constraint 3). On first mount, consume the hash
// fragment: navigate to it and clear the hash so a reload doesn't re-trigger.
// Runs once per window; a window opened with no initial route has no hash and
// this is a no-op.
function InitialRouteBootstrap() {
  const navigate = useNavigate();
  useEffect(() => {
    const hash = window.location.hash;
    if (hash.length > 1) {
      navigate(hash.slice(1), { replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return null;
}
