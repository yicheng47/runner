// Persistent shell: sidebar on the left, page content fills the rest.
//
// Used as a React Router layout route so the Sidebar mounts ONCE for the
// app's lifetime. Child routes render into `<Outlet />`. Without this,
// every page change tears down the sidebar and its `runner/activity`
// listener — and any event emitted during the brief reattach window
// (e.g., the activity event from `session_start_direct` triggered on
// the chat page's mount) gets lost, leaving the SESSION list stale.

import { useEffect, useState, type ReactNode } from "react";
import { Outlet } from "react-router-dom";

import { Sidebar } from "./Sidebar";
import { UpdateToast } from "./UpdateToast";
import {
  STORAGE_SIDEBAR_COLLAPSED,
  readStoredBool,
  writeStoredBool,
} from "../lib/settings";

export function AppShell({ children }: { children?: ReactNode }) {
  // Settings modal state hoisted here so both the Sidebar's bottom
  // Settings row and the UpdateToast's "Update" button can open it.
  // Toast → settings → download mirrors Quill's flow: the toast just
  // routes the user, the actual download/install lives in the pane.
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Sidebar collapsed/expanded lives at the shell so Cmd+S can toggle
  // it from anywhere in the app, not just when the sidebar is the
  // focused subtree.
  const [collapsed, setCollapsed] = useState<boolean>(() =>
    readStoredBool(STORAGE_SIDEBAR_COLLAPSED, false),
  );
  const [sidebarPreviewOpen, setSidebarPreviewOpen] = useState(false);
  // Single source of truth for persistence — both the Sidebar's
  // chevron click (setCollapsed via prop) and the Cmd+S keydown
  // funnel through `collapsed`, so one effect keeps localStorage in
  // sync. The harmless one-write at mount with the already-stored
  // value is fine.
  useEffect(() => {
    writeStoredBool(STORAGE_SIDEBAR_COLLAPSED, collapsed);
  }, [collapsed]);

  useEffect(() => {
    if (!collapsed) setSidebarPreviewOpen(false);
  }, [collapsed]);

  // Cmd+S (mac) / Ctrl+S (others) toggles the sidebar. Mirrors the
  // input-tag skip used by the ⌘K palette shortcut in Sidebar so we
  // don't hijack editing inside text fields. Cmd+\ remains as a
  // legacy alias for anyone who picked it up during development.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key !== "s" && e.key !== "\\") return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (tag === "input" || tag === "textarea") return;
      e.preventDefault();
      setCollapsed((prev) => !prev);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="relative flex h-screen overflow-hidden bg-bg text-fg">
      {collapsed ? (
        <div
          aria-hidden
          onMouseEnter={() => setSidebarPreviewOpen(true)}
          className="absolute left-0 top-0 z-30 h-full w-1"
        />
      ) : null}
      <Sidebar
        settingsOpen={settingsOpen}
        onSettingsOpenChange={setSettingsOpen}
        collapsed={collapsed}
        onCollapsedChange={setCollapsed}
        previewOpen={sidebarPreviewOpen}
        onPreviewOpenChange={setSidebarPreviewOpen}
      />
      <main className="relative flex flex-1 flex-col overflow-hidden">
        <div
          data-tauri-drag-region
          className="pointer-events-auto absolute left-0 right-0 top-0 z-10 h-7"
        />
        {children ?? <Outlet />}
      </main>
      <UpdateToast onOpenSettings={() => setSettingsOpen(true)} />
    </div>
  );
}
