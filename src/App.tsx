import { useEffect } from "react";
import {
  BrowserRouter,
  Routes,
  Route,
  Navigate,
  useLocation,
  useNavigate,
} from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { AppShell } from "./components/AppShell";
import { ToastProvider } from "./contexts/ToastContext";
import { UpdateProvider } from "./contexts/UpdateContext";
import { nudgeAppZoom } from "./lib/appZoom";
import { eventMatchesShortcut } from "./lib/keymap";
import { readAppZoom } from "./lib/settings";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";
import MissionWorkspace from "./pages/MissionWorkspace";
import Runners from "./pages/Runners";
import RunnerDetail from "./pages/RunnerDetail";
import RunnerChat from "./pages/RunnerChat";
import SettingsPage from "./pages/SettingsPage";

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
          <Routes>
            <Route element={<AppShell />}>
              <Route path="/" element={<Navigate to="/runners" replace />} />
              <Route path="/crews" element={<Crews />} />
              <Route path="/crews/:crewId" element={<CrewEditor />} />
              <Route path="/runners" element={<Runners />} />
              <Route path="/runners/:handle" element={<RunnerDetail />} />
              <Route path="/chats/:sessionId" element={<RunnerChat />} />
              <Route path="/missions/:id" element={<MissionWorkspace />} />
              <Route path="*" element={<Navigate to="/runners" replace />} />
            </Route>
            {/* Settings takes over the whole window — its own two-column
                surface without the app Sidebar (impl 0025). */}
            <Route path="/settings/:pane?" element={<SettingsPage />} />
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
