// Persistent shell: sidebar on the left, page content fills the rest.
//
// Used as a React Router layout route so the Sidebar mounts ONCE for the
// app's lifetime. Child routes render into `<Outlet />`. Without this,
// every page change tears down the sidebar and its `runner/activity`
// listener — and any event emitted during the brief reattach window
// (e.g., the activity event from `session_start_direct` triggered on
// the chat page's mount) gets lost, leaving the SESSION list stale.
//
// The chat surface and mission workspace don't render through the Outlet
// at all: PersistentSurfaces keeps them mounted across route changes so
// their terminals survive visits to the list pages (see that file).
//
// Settings (impl 0025) is a full-window takeover without the app
// Sidebar, but it must not unmount the shell: PersistentSurfaces lives
// in <main>, and unmounting it cold-remounts every terminal — the
// half-width-repaint path #309 removed for list pages. The chrome
// hides (display:none, subtree retained) under a Settings layer
// instead, and returning is the same active-flip the list pages use.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { matchPath, Outlet, useLocation } from "react-router-dom";

import { PersistentSurfaces } from "./PersistentSurfaces";
import { PanelToggleGlyph } from "./PanelToggleGlyph";
import { Sidebar } from "./Sidebar";
import SettingsPage from "../pages/SettingsPage";
import {
  STORAGE_SIDEBAR_COLLAPSED,
  readAppZoom,
  readStoredBool,
  writeStoredBool,
} from "../lib/settings";
import { syncTitlebarZoom } from "../lib/appZoom";
import { eventMatchesShortcut } from "../lib/keymap";
import { useCurrentWindowFullscreen } from "../hooks/useCurrentWindowFullscreen";

const SIDEBAR_TOGGLE_EVENT = "runner:toggle-sidebar";

export function AppShell({ children }: { children?: ReactNode }) {
  const location = useLocation();
  const syncWindowTitlebar = useCallback((nextFullscreen: boolean) => {
    if (!nextFullscreen) void syncTitlebarZoom(readAppZoom());
  }, []);
  const fullscreen = useCurrentWindowFullscreen(syncWindowTitlebar);
  const settingsActive =
    matchPath("/settings/:pane?", location.pathname) != null;
  const workspaceActive =
    matchPath("/chats/:sessionId", location.pathname) != null ||
    matchPath("/missions/:id", location.pathname) != null;
  // Sidebar collapsed/expanded lives at the shell so Cmd+S can toggle
  // it from anywhere in the app, not just when the sidebar is the
  // focused subtree.
  const [collapsed, setCollapsed] = useState<boolean>(() =>
    readStoredBool(STORAGE_SIDEBAR_COLLAPSED, false),
  );
  const [sidebarPreviewOpen, setSidebarPreviewOpen] = useState(false);
  // Single source of truth for persistence — both the Sidebar's
  // panel button (setCollapsed via prop) and the Cmd+S keydown
  // funnel through `collapsed`, so one effect keeps localStorage in
  // sync. The harmless one-write at mount with the already-stored
  // value is fine.
  useEffect(() => {
    writeStoredBool(STORAGE_SIDEBAR_COLLAPSED, collapsed);
  }, [collapsed]);

  useEffect(() => {
    if (!collapsed) setSidebarPreviewOpen(false);
  }, [collapsed]);

  // Both toggle listeners go inert while Settings covers the window —
  // the hidden chrome must not flip state the user can't see.
  useEffect(() => {
    if (settingsActive) return;
    const onToggle = () => setCollapsed((prev) => !prev);
    window.addEventListener(SIDEBAR_TOGGLE_EVENT, onToggle);
    return () => window.removeEventListener(SIDEBAR_TOGGLE_EVENT, onToggle);
  }, [settingsActive]);

  // Skip real text fields, but let xterm's hidden textarea through so
  // terminal focus doesn't swallow the app-level shortcut.
  useEffect(() => {
    if (settingsActive) return;
    const onKey = (e: KeyboardEvent) => {
      if (!eventMatchesShortcut(e, "toggle-sidebar")) return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      const isTextInput =
        tag === "input" || tag === "textarea" || !!target?.isContentEditable;
      const isXtermInput = !!target?.closest(".xterm");
      if (isTextInput && !isXtermInput) return;
      e.preventDefault();
      e.stopPropagation();
      window.dispatchEvent(new Event(SIDEBAR_TOGGLE_EVENT));
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [settingsActive]);

  return (
    <div className="relative flex h-screen overflow-hidden bg-bg text-fg">
      {/* `contents` keeps Sidebar and <main> direct flex items; `hidden`
          swaps to display:none under the Settings takeover without
          unmounting the subtree (terminals keep their buffers). */}
      <div className={settingsActive ? "hidden" : "contents"}>
        {collapsed ? (
          <div
            aria-hidden
            onMouseEnter={() => setSidebarPreviewOpen(true)}
            className="absolute left-0 top-0 z-30 h-full w-4"
          />
        ) : null}
        <Sidebar
          collapsed={collapsed}
          fullscreen={fullscreen}
          onCollapsedChange={setCollapsed}
          previewOpen={sidebarPreviewOpen}
          onPreviewOpenChange={setSidebarPreviewOpen}
        />
        <main className="relative flex flex-1 flex-col overflow-hidden">
          <div
            data-tauri-drag-region
            className="pointer-events-auto absolute left-0 right-0 top-0 z-10 h-7"
          />
          {collapsed && !workspaceActive ? (
            <div
              data-tauri-drag-region
              className={`absolute left-0 top-0 z-20 flex h-11 items-center ${
                fullscreen
                  ? "pl-2"
                  : "pl-[var(--titlebar-sidebar-toggle-gutter)]"
              }`}
            >
              <button
                type="button"
                onClick={() => setCollapsed(false)}
                title="Open sidebar (⌘S)"
                aria-label="Open sidebar"
                className="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg"
              >
                <PanelToggleGlyph
                  side="left"
                  filled={false}
                  className="h-[12px] w-[15.4px]"
                />
              </button>
            </div>
          ) : null}
          {children ?? <Outlet />}
          <PersistentSurfaces
            sidebarCollapsed={collapsed}
            fullscreen={fullscreen}
            onOpenSidebar={() => setCollapsed(false)}
          />
        </main>
      </div>
      {settingsActive ? (
        <div className="absolute inset-0 z-40">
          <SettingsPage />
        </div>
      ) : null}
    </div>
  );
}
