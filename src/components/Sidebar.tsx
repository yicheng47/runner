// App sidebar — Carbon & Plasma dark theme.
//
// Three sections, top to bottom:
//   - WORKSPACE: search (placeholder), runner, crew nav links.
//   - MISSION:   collapsible header with count + `+` (Start Mission), one row
//                per running mission. The currently-open mission is highlighted.
//   - SESSION:   collapsible header with count + `+` (jump to runners list),
//                one row per live direct-chat. The currently-open
//                direct chat is highlighted.
//
// MISSION pulls from `mission_list_summary` (filtered to status === "running").
// SESSION continues to consume `runner/activity` events for live direct chats.
// The two runtime sections refresh independently so a mission_start doesn't
// blink the direct-chat list and vice versa.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import {
  NavLink,
  useLocation,
  useMatch,
  useNavigate,
} from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  ChevronDown,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  MoreHorizontal,
  Pin,
  PinOff,
  Plus,
  Search,
  Settings as SettingsIcon,
  SquarePen,
  Terminal,
  Users,
} from "lucide-react";

import { api, type DirectSessionEntry } from "../lib/api";
import {
  markArchivingMission,
  markArchivingSession,
  unmarkArchivingMission,
  unmarkArchivingSession,
} from "../lib/archivingState";
import type { AppendedEvent, MissionSummary } from "../lib/types";
import { StartMissionModal } from "./StartMissionModal";
import { SettingsModal } from "./SettingsModal";
import { CommandPalette } from "./CommandPalette";

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 240;
const STORAGE_WIDTH = "runner.sidebar.width";
const STORAGE_MISSION_OPEN = "runner.sidebar.mission.open";
const STORAGE_SESSION_OPEN = "runner.sidebar.session.open";

function getStoredWidth(): number {
  if (typeof localStorage === "undefined") return SIDEBAR_DEFAULT;
  const stored = localStorage.getItem(STORAGE_WIDTH);
  if (stored) {
    const n = parseInt(stored, 10);
    if (!Number.isNaN(n) && n >= SIDEBAR_MIN && n <= SIDEBAR_MAX) return n;
  }
  return SIDEBAR_DEFAULT;
}

function getStoredFlag(key: string, fallback: boolean): boolean {
  if (typeof localStorage === "undefined") return fallback;
  const stored = localStorage.getItem(key);
  if (stored === "1") return true;
  if (stored === "0") return false;
  return fallback;
}

function setStoredFlag(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "1" : "0");
  } catch {
    // ignore quota / disabled-storage errors
  }
}

interface SidebarProps {
  // Settings open state lives in AppShell so the UpdateToast can also
  // open it (toast → settings → download flow). Passing the open
  // state down keeps the SettingsModal mounted here while letting
  // outsiders trigger it.
  settingsOpen: boolean;
  onSettingsOpenChange: (open: boolean) => void;
  // Collapsed/expanded state lives in AppShell so the global cmd+\
  // shortcut can toggle it. The `width` resize state stays local —
  // it's preserved across collapse/expand cycles so users get their
  // last full width back when they re-open.
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
}

export function Sidebar({
  settingsOpen,
  onSettingsOpenChange,
  collapsed,
  onCollapsedChange,
}: SidebarProps) {
  const navigate = useNavigate();
  const location = useLocation();

  // Width + resize state.
  const [width, setWidth] = useState<number>(getStoredWidth);

  // Runtime state.
  const [missions, setMissions] = useState<MissionSummary[]>([]);
  // Flat list of un-archived direct chats. Running ones first, then
  // stopped/crashed ordered by recency. Refreshed on session/exit and
  // runner/activity events. See docs/impls/0003-direct-chats.md.
  const [directSessions, setDirectSessions] = useState<DirectSessionEntry[]>(
    [],
  );

  // Section toggles, persisted so users don't have to re-expand each visit.
  const [missionsOpen, setMissionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_MISSION_OPEN, true),
  );
  const [sessionsOpen, setSessionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_SESSION_OPEN, true),
  );

  const [creatingMission, setCreatingMission] = useState(false);

  // Per-row context menu state. The Pencil design (P5CLA inside u6woG)
  // shows a floating menu with Pin / Rename / Archive next to a session
  // row. We anchor it by clientX/Y so right-click and ellipsis-button
  // both work without per-row refs. `null` = closed.
  const [sessionMenu, setSessionMenu] = useState<{
    session: DirectSessionEntry;
    x: number;
    y: number;
  } | null>(null);
  // Settings modal toggle. State now lives in AppShell so external
  // surfaces (e.g. UpdateToast) can also open it; we just mirror the
  // prop through a stable setter.
  const setSettingsOpen = onSettingsOpenChange;
  // Command palette toggle. Opened from the search nav row OR the
  // global ⌘K / Ctrl+K shortcut. Mirrors Pencil node `Fkoe8`.
  const [paletteOpen, setPaletteOpen] = useState(false);
  // Mission row context menu — same anchor model as sessionMenu.
  // Today's actions: Archive (real, calls mission_archive). Pin and
  // Rename are designed-in slots reserved for follow-ups.
  const [missionMenu, setMissionMenu] = useState<{
    mission: MissionSummary;
    x: number;
    y: number;
  } | null>(null);
  // Inline rename: when set, the row whose id matches renders an input
  // instead of its label. Submit (Enter) → session_rename + refresh.
  // Cancel (Escape / blur with no change) → close without write.
  const [renamingId, setRenamingId] = useState<string | null>(null);

  // Identify the currently-open runtime so we can highlight the matching
  // sidebar row. `useMatch` returns null when the URL doesn't match.
  const missionMatch = useMatch("/missions/:id");
  const currentMissionId = missionMatch?.params.id ?? null;
  const chatMatch = useMatch("/runners/:handle/chat/:sessionId");
  // Which direct-chat session is currently in view. The chat route
  // encodes the session id in the URL (a runner can host multiple
  // chats — see docs/impls/0003-direct-chats.md), so we match by
  // session id rather than handle.
  const currentChatSessionId = chatMatch?.params.sessionId ?? null;

  const refreshMissions = useCallback(async () => {
    try {
      const rows = await api.mission.listSummary();
      setMissions(rows.filter((m) => m.status === "running"));
    } catch (e) {
      // best-effort; the next event/refetch will resolve transient errors
      console.error("sidebar: refreshMissions failed", e);
    }
  }, []);

  // Mission tray: initial load + bus-driven refresh on mission_start /
  // mission_stopped envelopes. We don't filter by mission_id because the
  // sidebar must surface every running mission, not just the open one.
  useEffect(() => {
    void refreshMissions();
  }, [refreshMissions]);

  // ⌘K / Ctrl+K opens the command palette from anywhere in the app.
  // Skip when the user is editing inside an <input> or <textarea>
  // so the shortcut doesn't hijack normal text input.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "k" && e.key !== "K") return;
      if (!(e.metaKey || e.ctrlKey)) return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (tag === "input" || tag === "textarea") return;
      e.preventDefault();
      setPaletteOpen(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<AppendedEvent>("event/appended", (msg) => {
      const t = msg.payload.event.type;
      if (
        t === "mission_start" ||
        t === "mission_stopped" ||
        t === "ask_human" ||
        t === "human_question" ||
        t === "human_response"
      ) {
        // ask_human/human_question/human_response refresh the pending-ask
        // count badge. Cheap query; fires only on these signal types.
        void refreshMissions();
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshMissions]);

  // Direct-chat tray: pull the flat list of un-archived sessions and
  // refresh on lifecycle events.
  const refreshDirectSessions = useCallback(async () => {
    try {
      const rows = await api.session.listRecentDirect();
      setDirectSessions(rows);
    } catch (e) {
      console.error("sidebar: refreshDirectSessions failed", e);
    }
  }, []);

  useEffect(() => {
    void refreshDirectSessions();
  }, [refreshDirectSessions]);

  // session/exit fires when a running PTY reaps (live → stopped flip).
  // runner/activity fires on every spawn/reap and is our cue that a
  // new direct chat row may have appeared. Both refresh the same list.
  useEffect(() => {
    let unlistenExit: (() => void) | null = null;
    let unlistenActivity: (() => void) | null = null;
    let unlistenArchived: (() => void) | null = null;
    let unlistenUpdated: (() => void) | null = null;
    let cancelled = false;
    void Promise.all([
      listen("session/exit", () => {
        void refreshDirectSessions();
      }),
      listen("runner/activity", () => {
        void refreshDirectSessions();
      }),
      listen("session/archived", () => {
        // Fired by `session_archive` after the archived_at flip. Lets
        // the CHAT list drop the row whenever the user archives from
        // anywhere (sidebar's own Archive action already refreshes
        // explicitly, but RunnerChat's SessionEnded overlay relies on
        // this event since it doesn't own the sidebar's fetch).
        void refreshDirectSessions();
      }),
      listen("session/updated", () => {
        // Fired by `session_pin` and `session_rename` after the row
        // flips. Lets the CHAT list pick up pin/title changes that
        // came from the chat-page kebab — without this the sidebar
        // would show stale state until something else triggered a
        // refresh.
        void refreshDirectSessions();
      }),
    ]).then(([fnExit, fnActivity, fnArchived, fnUpdated]) => {
      if (cancelled) {
        fnExit();
        fnActivity();
        fnArchived();
        fnUpdated();
        return;
      }
      unlistenExit = fnExit;
      unlistenActivity = fnActivity;
      unlistenArchived = fnArchived;
      unlistenUpdated = fnUpdated;
    });
    return () => {
      cancelled = true;
      unlistenExit?.();
      unlistenActivity?.();
      unlistenArchived?.();
      unlistenUpdated?.();
    };
  }, [refreshDirectSessions]);

  const openMission = useCallback(
    (id: string) => {
      navigate(`/missions/${id}`);
    },
    [navigate],
  );

  // Open the per-row context menu (Pin / Rename / Archive) at the
  // pointer's position. Used by both right-click on the row and click
  // on the trailing ellipsis button. We clamp to the viewport in the
  // render path so the menu stays visible near the right edge.
  const openSessionMenu = useCallback(
    (session: DirectSessionEntry, anchor: { x: number; y: number }) => {
      setSessionMenu({ session, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeSessionMenu = useCallback(() => setSessionMenu(null), []);

  const openMissionMenu = useCallback(
    (mission: MissionSummary, anchor: { x: number; y: number }) => {
      setMissionMenu({ mission, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeMissionMenu = useCallback(() => setMissionMenu(null), []);

  const archiveMission = useCallback(
    async (mission: MissionSummary) => {
      markArchivingMission(mission.id);
      try {
        await api.mission.archive(mission.id);
        await refreshMissions();
        // If we just archived the mission the user was viewing,
        // bounce them off — the workspace will refuse to attach a
        // completed mission's router and the page will look broken.
        if (currentMissionId === mission.id) {
          navigate("/runners");
        }
      } catch (e) {
        console.error("sidebar: mission_archive failed", e);
      } finally {
        // Defer unmark past the navigate commit so the still-mounted
        // workspace doesn't briefly re-render with archivingMission=
        // false while React 18 batches the sync emit with the route
        // change. See archiveSession below for the full rationale.
        setTimeout(() => unmarkArchivingMission(mission.id), 0);
      }
    },
    [currentMissionId, navigate, refreshMissions],
  );

  const togglePinMission = useCallback(
    async (mission: MissionSummary) => {
      try {
        await api.mission.pin(mission.id, !mission.pinned_at);
        await refreshMissions();
      } catch (e) {
        console.error("sidebar: mission_pin failed", e);
      }
    },
    [refreshMissions],
  );

  // Track which mission row (if any) is currently in inline-rename
  // mode. Same pattern as session renames.
  const [renamingMissionId, setRenamingMissionId] = useState<string | null>(
    null,
  );
  const submitMissionRename = useCallback(
    async (id: string, nextTitle: string) => {
      const trimmed = nextTitle.trim();
      const original = missions.find((m) => m.id === id)?.title ?? "";
      setRenamingMissionId(null);
      if (!trimmed || trimmed === original) return;
      try {
        await api.mission.rename(id, trimmed);
        await refreshMissions();
      } catch (e) {
        console.error("sidebar: mission_rename failed", e);
      }
    },
    [missions, refreshMissions],
  );

  const togglePin = useCallback(
    async (session: DirectSessionEntry) => {
      try {
        await api.session.pin(session.session_id, !session.pinned);
        await refreshDirectSessions();
      } catch (e) {
        console.error("sidebar: session_pin failed", e);
      }
    },
    [refreshDirectSessions],
  );

  const archiveSession = useCallback(
    async (session: DirectSessionEntry) => {
      // Mark before the kill so the pill appears immediately on click —
      // session_kill awaits a 200ms grace + reader join in the backend
      // and the user shouldn't see a frozen UI in the meantime.
      markArchivingSession(session.session_id);
      // Backend refuses to archive a running session; kill first if
      // the user explicitly chose Archive on a live row.
      try {
        if (session.status === "running") {
          await api.session.kill(session.session_id);
        }
        await api.session.archive(session.session_id);
        await refreshDirectSessions();
        if (currentChatSessionId === session.session_id) {
          navigate(`/runners/${session.handle}`);
        }
      } catch (e) {
        console.error("sidebar: session_archive failed", e);
      } finally {
        // Defer unmark past the navigate commit so RunnerChat doesn't
        // briefly re-render with archiving=false while still mounted.
        // React 18 batches the sync emit (useSyncExternalStore) with
        // the route change, so without the defer the chat lands one
        // render with chatState="stopped" + archiving=false → its
        // overlay branch falls through to SessionEndedOverlay,
        // flashing the "Resume @handle" popup before the unmount.
        // Same shape applies to archiveMission above and archiveChat
        // in RunnerChat — keep all three defers in sync.
        setTimeout(() => unmarkArchivingSession(session.session_id), 0);
      }
    },
    [currentChatSessionId, navigate, refreshDirectSessions],
  );

  const submitRename = useCallback(
    async (sessionId: string, nextTitle: string | null) => {
      try {
        await api.session.rename(sessionId, nextTitle);
        await refreshDirectSessions();
      } catch (e) {
        console.error("sidebar: session_rename failed", e);
      } finally {
        setRenamingId(null);
      }
    },
    [refreshDirectSessions],
  );

  // Click on a SESSION row — always just navigate to the chat. The
  // chat surface owns the running/stopped UI: a stopped session lands
  // on a dimmed terminal with a "Session ended" overlay, and the user
  // explicitly clicks **Resume** there to bring the PTY back. Earlier
  // we auto-resumed on click, but that conflated "I want to look at
  // this chat" with "I want to relaunch the agent" — the explicit
  // Resume affordance avoids accidental respawns.
  //
  // We pass `sessionStatus` through navigation state so RunnerChat's
  // attach path can seed the pane with the row's real status. Without
  // it, the pane briefly renders as running and xterm can forward a
  // keystroke to `session_inject_stdin` for a session that's no
  // longer in the live map → "session not found" banner.
  const openDirectChat = useCallback(
    (entry: DirectSessionEntry) => {
      const target = `/runners/${entry.handle}/chat/${entry.session_id}`;
      navigate(target, {
        state: { sessionStatus: entry.status },
        replace: location.pathname === target,
      });
    },
    [navigate, location.pathname],
  );

  // SESSION's `+` button — direct chats are spawned from a runner, so we
  // route to the runners list and let the user pick. A future v0.x could
  // open an inline runner-picker popover instead.
  const handleNewDirectChat = useCallback(() => {
    navigate("/runners");
  }, [navigate]);

  const toggleMissions = useCallback(() => {
    setMissionsOpen((prev) => {
      setStoredFlag(STORAGE_MISSION_OPEN, !prev);
      return !prev;
    });
  }, []);

  const toggleSessions = useCallback(() => {
    setSessionsOpen((prev) => {
      setStoredFlag(STORAGE_SESSION_OPEN, !prev);
      return !prev;
    });
  }, []);

  // Drag-to-resize handle on the right edge — same logic as before.
  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = width;
      const onMouseMove = (ev: MouseEvent) => {
        const next = Math.min(
          SIDEBAR_MAX,
          Math.max(SIDEBAR_MIN, startWidth + ev.clientX - startX),
        );
        setWidth(next);
      };
      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        setWidth((w) => {
          try {
            localStorage.setItem(STORAGE_WIDTH, String(w));
          } catch {
            // ignore quota / disabled-storage errors
          }
          return w;
        });
      };
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [width],
  );

  return (
    <>
      <aside
        style={{ width: collapsed ? 52 : width }}
        className="relative flex h-full shrink-0 select-none flex-col overflow-hidden border-r border-line bg-raised transition-[width] duration-150"
      >
        <div data-tauri-drag-region className="h-7" />

        {collapsed ? (
          // Rail body: single flex column. The h-7 drag strip above
          // already contributes 28px of top padding; pt-2 brings the
          // brand to y=36 to match the .pen frame's [36, 0, 16, 0]
          // padding. gap-1 (4px) between siblings; explicit h-2
          // spacer adds the 8px gap below the divider that the
          // designer called for.
          <div className="flex min-h-0 flex-1 flex-col items-center gap-1 pb-4 pt-2">
            <BrandMark />
            <div className="h-px w-full bg-line" />
            <div className="h-2 w-full" />
            <RailIconLink icon={Terminal} to="/runners" label="runner" />
            <RailIconLink icon={Users} to="/crews" label="crew" />
            <RailIconButton
              icon={Search}
              label="Search"
              onClick={() => setPaletteOpen(true)}
            />
            {/* Archived rail icon — slot reserved for feature #31.
                When the archived nav lands, render a RailIconLink
                here pointing at its route (icon: lucide Archive).
                Leaving this as a clearly-marked placeholder so the
                merge with #31 is mechanical. */}
            <div className="w-full flex-1" />
            <div className="h-px w-full bg-line" />
            <button
              type="button"
              onClick={() => onCollapsedChange(false)}
              title="Expand sidebar (⌘\\)"
              aria-label="Expand sidebar"
              className="flex h-9 w-9 cursor-pointer items-center justify-center rounded border border-transparent text-fg-2 transition-colors hover:border-line hover:bg-bg hover:text-fg"
            >
              <ChevronsRight aria-hidden className="h-4 w-4" />
            </button>
            <RailIconButton
              icon={SettingsIcon}
              label="Settings"
              onClick={() => setSettingsOpen(true)}
            />
          </div>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col pb-4">
            {/* Brand row — open state only. Toggle has moved to the
                bottom Settings row, so this is brand + wordmark
                only. */}
            <div className="flex shrink-0 items-center gap-2 px-5 pb-5 pt-1">
              <BrandMark />
              <span className="text-base font-semibold tracking-tight text-fg">
                Runner
              </span>
            </div>
            {/* WORKSPACE keeps natural height; doesn't compete for the
                flex-share allotted to MISSION + SESSION. */}
            <div className="shrink-0">
              <SectionHeader>WORKSPACE</SectionHeader>
              <nav className="flex flex-col gap-0.5 px-3 pb-1">
                <NavRow icon={Terminal} to="/runners" label="runner" />
                <NavRow icon={Users} to="/crews" label="crew" />
                {/* Search opens a command-palette modal — matches design
                    `Fkoe8`. Default interaction is click-to-callout, not
                    type-in-place, so this lives as a nav row alongside
                    runner/crew rather than an inline input. The actual
                    palette is a follow-up; for now the row stubs the
                    callout. */}
                <SearchNavRow onOpen={() => setPaletteOpen(true)} />
              </nav>
            </div>

            <div className="h-5 shrink-0" />

            {/* MISSION + SESSION always split the remaining vertical
                space 1:2 (mission takes 1 share, session takes 2),
                regardless of expand/collapse state. Collapsing a
                section just hides its body — the section still claims
                its share of height so the column rhythm doesn't jump
                when toggling. Each expanded body scrolls independently
                so a long SESSION list can't push MISSION off-screen. */}
            <section className="flex min-h-0 flex-[1] basis-0 flex-col">
              <CollapsibleSectionHeader
                label="MISSION"
                count={missions.length}
                open={missionsOpen}
                onToggle={toggleMissions}
                onPlus={() => setCreatingMission(true)}
                plusTitle="Start mission"
              />
              {missionsOpen ? (
                <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-3 pt-1">
                  {missions.length === 0 ? (
                    <p className="px-2.5 py-1 text-xs text-fg-3">
                      No live missions.
                    </p>
                  ) : (
                    missions.map((m) => (
                      <RuntimeRow
                        key={m.id}
                        selected={m.id === currentMissionId}
                        label={m.title}
                        onClick={() => openMission(m.id)}
                        onContextMenu={(anchor) => openMissionMenu(m, anchor)}
                        title={m.crew_name || ""}
                        pinned={!!m.pinned_at}
                        renaming={renamingMissionId === m.id}
                        onRenameSubmit={(next) =>
                          void submitMissionRename(m.id, next)
                        }
                        onRenameCancel={() => setRenamingMissionId(null)}
                      />
                    ))
                  )}
                </div>
              ) : null}
            </section>

            <div className="h-8 shrink-0" />

            <section className="flex min-h-0 flex-[2] basis-0 flex-col">
              <CollapsibleSectionHeader
                label="CHAT"
                count={directSessions.length}
                open={sessionsOpen}
                onToggle={toggleSessions}
                onPlus={handleNewDirectChat}
                plusTitle="Start a chat"
              />
              {sessionsOpen ? (
                <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-3 pt-1">
                  {directSessions.length === 0 ? (
                    <p className="px-2.5 py-1 text-xs text-fg-3">
                      No chats yet.
                    </p>
                  ) : (
                    directSessions.map((s) => (
                      <SessionRow
                        key={s.session_id}
                        session={s}
                        selected={s.session_id === currentChatSessionId}
                        renaming={renamingId === s.session_id}
                        onClick={() => openDirectChat(s)}
                        onContextMenu={(anchor) => openSessionMenu(s, anchor)}
                        onRenameSubmit={(nextTitle) =>
                          void submitRename(s.session_id, nextTitle)
                        }
                        onRenameCancel={() => setRenamingId(null)}
                      />
                    ))
                  )}
                </div>
              ) : null}
            </section>

            {/* Settings row — pinned at the bottom of the sidebar
                column. Mirrors Pencil node `IJsUO` (sidebar settings).
                The collapse toggle now lives on the right of this
                row (24×24, ChevronsLeft) — see frame s31wik. The
                Settings button keeps the rest of the row width via
                flex-1 so its hit target stays large. */}
            <div className="flex shrink-0 items-center gap-2 border-t border-line px-3 pt-2">
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className="flex flex-1 cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-fg-2 transition-colors hover:bg-raised hover:text-fg"
              >
                <SettingsIcon aria-hidden className="h-3.5 w-3.5" />
                <span className="text-[13px]">Settings</span>
              </button>
              <button
                type="button"
                onClick={() => onCollapsedChange(true)}
                title="Collapse sidebar (⌘\\)"
                aria-label="Collapse sidebar"
                className="flex h-6 w-6 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-bg hover:text-fg"
              >
                <ChevronsLeft aria-hidden className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        )}

        {/* Resize handle is inert in rail mode (no width to drag). We
            render a noop placeholder so the DOM stays stable, just
            without the col-resize cursor / mousedown handler. */}
        <div
          onMouseDown={collapsed ? undefined : handleResizeStart}
          title={collapsed ? undefined : "Drag to resize"}
          className={
            collapsed
              ? "absolute right-0 top-0 z-20 h-full w-1 bg-transparent"
              : "absolute right-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
          }
        />
      </aside>

      <SettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
      />

      <StartMissionModal
        open={creatingMission}
        onClose={() => setCreatingMission(false)}
        onStarted={(mission) => {
          setCreatingMission(false);
          void refreshMissions();
          navigate(`/missions/${mission.id}`);
        }}
      />

      {sessionMenu ? (
        <SessionContextMenu
          session={sessionMenu.session}
          anchorX={sessionMenu.x}
          anchorY={sessionMenu.y}
          onClose={closeSessionMenu}
          onPin={() => {
            void togglePin(sessionMenu.session);
            closeSessionMenu();
          }}
          onRename={() => {
            setRenamingId(sessionMenu.session.session_id);
            closeSessionMenu();
          }}
          onArchive={() => {
            void archiveSession(sessionMenu.session);
            closeSessionMenu();
          }}
        />
      ) : null}

      {missionMenu ? (
        <MissionContextMenu
          pinned={!!missionMenu.mission.pinned_at}
          anchorX={missionMenu.x}
          anchorY={missionMenu.y}
          onClose={closeMissionMenu}
          onPin={() => {
            void togglePinMission(missionMenu.mission);
            closeMissionMenu();
          }}
          onRename={() => {
            setRenamingMissionId(missionMenu.mission.id);
            closeMissionMenu();
          }}
          onArchive={() => {
            void archiveMission(missionMenu.mission);
            closeMissionMenu();
          }}
        />
      ) : null}
    </>
  );
}

// ---- nav rows ----------------------------------------------------------

function NavRow({
  icon: Icon,
  to,
  label,
}: {
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  to: string;
  label: string;
}) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        `flex items-center gap-2 rounded px-2.5 py-1.5 text-sm transition-colors ${
          isActive
            ? "bg-bg font-semibold text-fg"
            : "text-fg-2 hover:text-fg"
        }`
      }
    >
      {({ isActive }) => (
        <>
          <Icon
            aria-hidden
            className={`h-3 w-3 ${isActive ? "text-fg" : "text-fg-2"}`}
          />
          <span>{label}</span>
        </>
      )}
    </NavLink>
  );
}

// Icon-only nav row used in the collapsed rail. Reuses NavLink for
// the active-state styling so the rail and full sidebar agree on
// what's selected. The label is exposed via a native `title`
// tooltip — there is no in-tree tooltip primitive, so we keep this
// lightweight rather than introduce one for v1.
function RailIconLink({
  icon: Icon,
  to,
  label,
}: {
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  to: string;
  label: string;
}) {
  return (
    <NavLink
      to={to}
      title={label}
      aria-label={label}
      className={({ isActive }) =>
        `flex h-9 w-9 items-center justify-center rounded border transition-colors ${
          isActive
            ? "border-line bg-bg text-fg"
            : "border-transparent text-fg-2 hover:border-line hover:bg-bg hover:text-fg"
        }`
      }
    >
      <Icon aria-hidden className="h-4 w-4" />
    </NavLink>
  );
}

// Icon-only button variant for non-routing actions in the rail
// (search palette, settings). Border-transparent → border-line on
// hover keeps the layout stable while matching the active treatment
// on the link variant.
function RailIconButton({
  icon: Icon,
  label,
  onClick,
}: {
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className="flex h-9 w-9 cursor-pointer items-center justify-center rounded border border-transparent text-fg-2 transition-colors hover:border-line hover:bg-bg hover:text-fg"
    >
      <Icon aria-hidden className="h-4 w-4" />
    </button>
  );
}

/// Search nav row — visually indistinguishable from runner/crew rows
/// but opens the CommandPalette modal instead of routing. The ⌘K
/// keyboard binding (registered above) still works; the shortcut
/// hint just isn't displayed in the UI.
function SearchNavRow({ onOpen }: { onOpen: () => void }) {
  return (
    <button
      type="button"
      title="Search"
      onClick={onOpen}
      className="flex w-full cursor-pointer items-center gap-2 rounded px-2.5 py-1.5 text-left text-sm text-fg-2 transition-colors hover:text-fg"
    >
      <Search aria-hidden className="h-3 w-3 text-fg-2" />
      <span>search</span>
    </button>
  );
}

// ---- collapsible section header ---------------------------------------

function CollapsibleSectionHeader({
  label,
  count,
  open,
  onToggle,
  onPlus,
  plusTitle,
}: {
  label: string;
  count: number;
  open: boolean;
  onToggle: () => void;
  onPlus: () => void;
  plusTitle: string;
}) {
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div className="flex items-center justify-between gap-2 px-5 pb-1.5">
      <button
        type="button"
        onClick={onToggle}
        className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3 hover:text-fg-2"
      >
        <Chevron aria-hidden className="h-2.5 w-2.5" />
        <span>{label}</span>
        {count > 0 ? (
          <span className="ml-1 rounded-full bg-bg px-1.5 py-px font-mono text-[10px] font-semibold text-fg-3">
            {count}
          </span>
        ) : null}
      </button>
      <button
        type="button"
        onClick={onPlus}
        title={plusTitle}
        aria-label={plusTitle}
        className="cursor-pointer rounded p-1 text-fg-2 transition-colors hover:bg-bg hover:text-fg"
      >
        <Plus aria-hidden className="h-3 w-3" />
      </button>
    </div>
  );
}

// ---- runtime row (mission or direct-session) --------------------------

function RuntimeRow({
  selected,
  label,
  onClick,
  onContextMenu,
  title,
  mono,
  dim,
  pinned,
  renaming,
  onRenameSubmit,
  onRenameCancel,
}: {
  selected: boolean;
  label: string;
  onClick: () => void;
  /** Right-click handler. Anchor the menu at clientX/clientY. */
  onContextMenu?: (anchor: { x: number; y: number }) => void;
  title?: string;
  mono?: boolean;
  /** True when the row represents a non-running runtime (e.g. a stopped
   *  direct chat that can be resumed). Mutes the status dot so the user
   *  can tell which sessions are live at a glance. */
  dim?: boolean;
  /** Pinned rows show a Pin icon next to the label. */
  pinned?: boolean;
  /** When true, replaces the label with an inline rename input. */
  renaming?: boolean;
  onRenameSubmit?: (next: string) => void;
  onRenameCancel?: () => void;
}) {
  if (renaming && onRenameSubmit && onRenameCancel) {
    return (
      <RowRenameInput
        initial={label}
        mono={mono}
        onSubmit={onRenameSubmit}
        onCancel={onRenameCancel}
      />
    );
  }
  return (
    <div
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault();
              onContextMenu({ x: e.clientX, y: e.clientY });
            }
          : undefined
      }
      className={`group flex w-full items-center gap-2 rounded border px-2.5 py-1.5 text-left text-xs transition-colors ${
        selected
          ? "border-line bg-bg text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      <button
        type="button"
        onClick={onClick}
        title={title}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 text-left"
      >
        <span
          className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
            dim ? "bg-fg-3" : "bg-accent"
          }`}
        />
        <span className={`truncate flex-1 ${mono ? "font-mono" : ""}`}>
          {label}
        </span>
        {pinned ? (
          <Pin aria-hidden className="h-3 w-3 shrink-0 text-fg-3" />
        ) : null}
      </button>
      {/* Kebab anchor for the same context menu the row's
          right-click triggers. Mirrors SessionRow's affordance so
          mission rows get a discoverable "..." button on hover —
          right-click alone isn't an obvious entry point. */}
      {onContextMenu ? (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onContextMenu({ x: e.clientX, y: e.clientY });
          }}
          title="More actions"
          aria-label="More actions"
          className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 transition-opacity hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
        >
          <MoreHorizontal aria-hidden className="h-3 w-3" />
        </button>
      ) : null}
    </div>
  );
}

function RowRenameInput({
  initial,
  mono,
  onSubmit,
  onCancel,
}: {
  initial: string;
  mono?: boolean;
  onSubmit: (next: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);
  return (
    <div className="flex w-full items-center gap-2 rounded border border-line bg-bg px-2.5 py-1 text-xs">
      <span className="inline-flex h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
      <input
        ref={inputRef}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onSubmit(draft);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (draft.trim() === initial.trim()) onCancel();
          else onSubmit(draft);
        }}
        className={`min-w-0 flex-1 bg-transparent text-fg outline-none placeholder:text-fg-3 ${
          mono ? "font-mono" : ""
        }`}
      />
    </div>
  );
}

// SESSION row: thin wrapper around RuntimeRow that adds (a) a trailing
// ellipsis button to open the per-row context menu, (b) right-click on
// the whole row as the same affordance, (c) an inline rename input
// that swaps in for the label while editing, and (d) a pin glyph for
// pinned rows. Mirrors the Pencil design `P5CLA` inside `u6woG`.
function SessionRow({
  session,
  selected,
  renaming,
  onClick,
  onContextMenu,
  onRenameSubmit,
  onRenameCancel,
}: {
  session: DirectSessionEntry;
  selected: boolean;
  renaming: boolean;
  onClick: () => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  onRenameSubmit: (nextTitle: string | null) => void;
  onRenameCancel: () => void;
}) {
  const defaultLabel = `@${session.handle} · ${formatStartedAt(session)}`;
  const label = session.title ?? defaultLabel;
  const dim = session.status !== "running";
  const tooltip = `@${session.handle} · ${session.status}${
    session.status !== "running" && session.resumable ? " · resumable" : ""
  }${session.pinned ? " · pinned" : ""}`;

  if (renaming) {
    // Inline rename input: pre-fills with the current label, submits
    // on Enter, cancels on Escape or blur. Empty input means clear back
    // to the auto-derived label.
    return (
      <div
        className="flex items-center gap-2 rounded border border-line bg-bg px-2.5 py-1.5"
        title={tooltip}
      >
        <span
          className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
            dim ? "bg-fg-3" : "bg-accent"
          }`}
        />
        <input
          autoFocus
          defaultValue={session.title ?? ""}
          placeholder={defaultLabel}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              const next = (e.target as HTMLInputElement).value.trim();
              onRenameSubmit(next.length === 0 ? null : next);
            } else if (e.key === "Escape") {
              e.preventDefault();
              onRenameCancel();
            }
          }}
          onBlur={(e) => {
            const next = e.target.value.trim();
            const prior = session.title ?? "";
            if (next === prior.trim()) {
              onRenameCancel();
            } else {
              onRenameSubmit(next.length === 0 ? null : next);
            }
          }}
          className="flex-1 truncate bg-transparent font-mono text-xs text-fg outline-none placeholder:text-fg-3"
        />
      </div>
    );
  }

  return (
    <div
      className={`group flex w-full items-center gap-2 rounded border px-2.5 py-1.5 text-left text-xs transition-colors ${
        selected
          ? "border-line bg-bg text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      <button
        type="button"
        onClick={onClick}
        title={tooltip}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
      >
        <span
          className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
            dim ? "bg-fg-3" : "bg-accent"
          }`}
        />
        {session.pinned ? (
          <Pin
            aria-hidden
            className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3"
          />
        ) : null}
        <span className="truncate font-mono">{label}</span>
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onContextMenu({ x: e.clientX, y: e.clientY });
        }}
        title="More actions"
        aria-label="More actions"
        className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 transition-opacity hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
      >
        <MoreHorizontal aria-hidden className="h-3 w-3" />
      </button>
    </div>
  );
}

// Floating action menu anchored at (anchorX, anchorY). Mirrors the
// Pencil design `P5CLA`: 140px wide, 6px padding, 1px gap, lucide
// icons, dark surface with a subtle drop shadow. Closes on outside
// click, Escape, or any of its actions firing.
function SessionContextMenu({
  session,
  anchorX,
  anchorY,
  onClose,
  onPin,
  onRename,
  onArchive,
}: {
  session: DirectSessionEntry;
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onArchive: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: anchorX, y: anchorY });

  // Clamp to viewport so the menu doesn't run off the right or bottom
  // edge. Measure after mount, then translate if needed.
  useEffect(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const margin = 4;
    const x = Math.min(anchorX, vw - rect.width - margin);
    const y = Math.min(anchorY, vh - rect.height - margin);
    setPos({ x: Math.max(margin, x), y: Math.max(margin, y) });
  }, [anchorX, anchorY]);

  // Outside-click + Escape close.
  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const isPinned = session.pinned;
  return (
    <div
      ref={ref}
      role="menu"
      style={{ position: "fixed", left: pos.x, top: pos.y, width: 140 }}
      className="z-50 flex flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
    >
      <ContextMenuItem
        icon={isPinned ? PinOff : Pin}
        label={isPinned ? "Unpin" : "Pin"}
        onClick={onPin}
      />
      <ContextMenuItem icon={SquarePen} label="Rename" onClick={onRename} />
      <ContextMenuItem icon={Archive} label="Archive" onClick={onArchive} />
    </div>
  );
}

function ContextMenuItem({
  icon: Icon,
  label,
  onClick,
  disabled,
  danger,
}: {
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      className={`flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] hover:bg-line disabled:cursor-default disabled:opacity-50 disabled:hover:bg-transparent ${
        danger ? "text-danger" : "text-fg"
      }`}
    >
      <Icon
        aria-hidden
        className={`h-3.5 w-3.5 ${danger ? "text-danger" : "text-fg"}`}
      />
      <span>{label}</span>
    </button>
  );
}

/// Mission row context menu — Pin, Rename, Archive. Layout matches
/// Pencil node `EWpGa` in `runners-design.pen`.
function MissionContextMenu({
  pinned,
  anchorX,
  anchorY,
  onClose,
  onPin,
  onRename,
  onArchive,
}: {
  pinned: boolean;
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onArchive: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: anchorX, y: anchorY });

  useEffect(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    const margin = 4;
    const x = Math.min(anchorX, window.innerWidth - rect.width - margin);
    const y = Math.min(anchorY, window.innerHeight - rect.height - margin);
    setPos({ x: Math.max(margin, x), y: Math.max(margin, y) });
  }, [anchorX, anchorY]);

  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      style={{ position: "fixed", left: pos.x, top: pos.y, width: 160 }}
      className="z-50 flex flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
    >
      <ContextMenuItem
        icon={pinned ? PinOff : Pin}
        label={pinned ? "Unpin" : "Pin"}
        onClick={onPin}
      />
      <ContextMenuItem icon={SquarePen} label="Rename" onClick={onRename} />
      <ContextMenuItem
        icon={Archive}
        label="Archive"
        onClick={onArchive}
        danger
      />
    </div>
  );
}

// Cheap relative-ish label for sessions that have no user-set title.
// Prefers the started_at column; falls back to stopped_at if both are
// set (older rows stay sortable). Months are short to keep the row narrow.
function formatStartedAt(s: DirectSessionEntry): string {
  const ts = s.started_at ?? s.stopped_at;
  if (!ts) return "session";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "session";
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// ---- chrome ------------------------------------------------------------

function BrandMark() {
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      aria-hidden
      className="shrink-0"
    >
      <ChevronGlyph x={3} y={3} size={9} opacity={0.4} />
      <ChevronGlyph x={9} y={9} size={14} opacity={1} />
      <ChevronGlyph x={3} y={20} size={9} opacity={0.4} />
    </svg>
  );
}

function ChevronGlyph({
  x,
  y,
  size,
  opacity,
}: {
  x: number;
  y: number;
  size: number;
  opacity: number;
}) {
  return (
    <svg x={x} y={y} width={size} height={size} viewBox="0 0 24 24">
      <polyline
        points="9 18 15 12 9 6"
        fill="none"
        stroke="#00FF9C"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={opacity}
      />
    </svg>
  );
}

function SectionHeader({ children }: { children: ReactNode }) {
  return (
    <div className="px-5 pb-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
      {children}
    </div>
  );
}
