// Keep-alive host for the two terminal-bearing surfaces (impl 0018 shell,
// chat surface + mission workspace).
//
// React Router unmounts route elements on navigation, which for these
// surfaces means disposing every xterm instance and replaying PTY
// snapshots on return. That cold-remount path has to re-fit geometry
// mid-layout, and a fit against a not-yet-settled rect pushes wrong cols
// to the PTY — the agent repaints at ~half width and the ring purge makes
// that narrow frame the only snapshot content. Rather than hardening the
// remount path further, the surfaces stay mounted: this host renders the
// last-visited chat surface and mission workspace as display:none layers
// while list pages are shown — the same mechanism ChatPaneGroup already
// uses for hidden panes inside the surface. Returning to a chat or
// mission is then an `active` flip on a live terminal (the tab-return
// path) instead of a cold remount.
//
// The `visible` prop gates what a hidden surface must not do: report
// window subjects (impl 0018 ownership), hold global shortcut listeners
// (both surfaces listen for RUNNER_TERMINAL_CYCLE_EVENT — an ungated
// hidden listener would double-handle it), and keep terminals `active`
// (hidden panes release their WebGL context and skip geometry pushes).
//
// Retention is bounded: at most one chat surface and one mission
// workspace, keyed to the last-visited id. Settings is a full-window
// takeover but renders through AppShell's takeover layer (this host
// stays mounted underneath), so a Settings round-trip is the same
// hide/show flip as a list-page visit.

import { useEffect, useState } from "react";
import { matchPath, useLocation } from "react-router-dom";

import MissionWorkspace from "../pages/MissionWorkspace";
import RunnerChat from "../pages/RunnerChat";

export function PersistentSurfaces({
  sidebarCollapsed,
  fullscreen,
  onOpenSidebar,
}: {
  sidebarCollapsed: boolean;
  fullscreen: boolean;
  onOpenSidebar: () => void;
}) {
  const location = useLocation();
  const chatId =
    matchPath("/chats/:sessionId", location.pathname)?.params.sessionId ??
    null;
  const missionId =
    matchPath("/missions/:id", location.pathname)?.params.id ?? null;

  const [lastChatId, setLastChatId] = useState<string | null>(null);
  const [lastMissionId, setLastMissionId] = useState<string | null>(null);
  useEffect(() => {
    if (chatId) setLastChatId(chatId);
  }, [chatId]);
  useEffect(() => {
    if (missionId) setLastMissionId(missionId);
  }, [missionId]);

  const mountedChatId = chatId ?? lastChatId;
  const mountedMissionId = missionId ?? lastMissionId;

  // `contents` keeps each page root a direct flex item of AppShell's
  // <main>, identical to rendering through <Outlet />.
  return (
    <>
      {mountedChatId !== null ? (
        <div className={chatId !== null ? "contents" : "hidden"}>
          <RunnerChat
            sessionId={mountedChatId}
            visible={chatId !== null}
            sidebarCollapsed={sidebarCollapsed}
            fullscreen={fullscreen}
            onOpenSidebar={onOpenSidebar}
          />
        </div>
      ) : null}
      {mountedMissionId !== null ? (
        <div className={missionId !== null ? "contents" : "hidden"}>
          <MissionWorkspace
            missionId={mountedMissionId}
            visible={missionId !== null}
            sidebarCollapsed={sidebarCollapsed}
            fullscreen={fullscreen}
            onOpenSidebar={onOpenSidebar}
          />
        </div>
      ) : null}
    </>
  );
}
