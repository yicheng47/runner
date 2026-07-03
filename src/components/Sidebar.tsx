// App sidebar — Carbon & Plasma dark theme.
//
// Three sections, top to bottom:
//   - WORKSPACE: search (placeholder), runner, crew nav links.
//   - MISSION:   collapsible header with count + `+` (Start Mission), one row
//                per running mission. The currently-open mission is highlighted.
//   - SESSION:   collapsible header with count + `+` (opens the
//                StartChat modal — runner pick + optional chat name +
//                working dir), one row per live direct-chat. The
//                currently-open direct chat is highlighted.
//
// MISSION pulls from `mission_list_summary` (filtered to status === "running").
// SESSION continues to consume `runner/activity` events for live direct chats.
// The two runtime sections refresh independently so a mission_start doesn't
// blink the direct-chat list and vice versa.

import {
  useCallback,
  useEffect,
  useMemo,
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
  AppWindow,
  Archive,
  ChevronDown,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  MessageSquarePlus,
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
import { useT, type TFn } from "../lib/i18n";
import { MOD_LABEL, isMac } from "../lib/platform";
import {
  markArchivingMission,
  markArchivingSession,
  unmarkArchivingMission,
  unmarkArchivingSession,
} from "../lib/archivingState";
import { useResizableWidth } from "../hooks/useResizableWidth";
import {
  BRAND_MARK_PINNED_COLOR,
  readBrandTint,
  STORAGE_APP_BRAND_TINT,
} from "../lib/settings";
import type {
  AppendedEvent,
  MissionActivityState,
  MissionSummary,
  SessionActivityEvent,
  SessionActivityState,
} from "../lib/types";
import { StartMissionModal } from "./StartMissionModal";
import { StartChatModal } from "./StartChatModal";
import { SettingsModal } from "./SettingsModal";
import { CommandPalette } from "./CommandPalette";

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 240;
const STORAGE_WIDTH = "runner.sidebar.width";
const STORAGE_MISSION_OPEN = "runner.sidebar.mission.open";
const STORAGE_SESSION_OPEN = "runner.sidebar.session.open";
const SIDEBAR_NAVIGATE_EVENT = "runner:navigate-sidebar-page";
const SIDEBAR_NAVIGATION_HISTORY_LIMIT = 64;

type SidebarNavigationDirection = "previous" | "next";

interface SidebarNavigationEntry {
  to: string;
  state?: { sessionStatus: DirectSessionEntry["status"] };
}

interface SidebarNavigationHistory {
  entries: string[];
  index: number;
}

function sidebarNavigationDirectionFromKey(
  e: KeyboardEvent,
): SidebarNavigationDirection | null {
  if (!(e.metaKey || e.ctrlKey)) return null;
  if (e.altKey || e.shiftKey) return null;
  if (e.key === "[" || e.code === "BracketLeft") return "previous";
  if (e.key === "]" || e.code === "BracketRight") return "next";
  return null;
}

function sidebarRuntimeKeyForPath(pathname: string): string | null {
  if (pathname.startsWith("/missions/") || pathname.startsWith("/chats/")) {
    return pathname;
  }
  return null;
}

function isSidebarNavigationDirection(
  value: unknown,
): value is SidebarNavigationDirection {
  return value === "previous" || value === "next";
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
  // Collapsed/expanded state lives in AppShell so the global Cmd+S
  // shortcut can toggle it. The `width` resize state stays local —
  // it's preserved across collapse/expand cycles so users get their
  // last full width back when they re-open.
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  previewOpen: boolean;
  onPreviewOpenChange: (open: boolean) => void;
}

export function Sidebar({
  settingsOpen,
  onSettingsOpenChange,
  collapsed,
  onCollapsedChange,
  previewOpen,
  onPreviewOpenChange,
}: SidebarProps) {
  const t = useT();
  const navigate = useNavigate();
  const location = useLocation();

  // Width + resize state. The right-edge handle below grows the
  // sidebar when dragged rightward. The aside ref lets the hook
  // write style.width directly during drag instead of going through
  // setState per mousemove (avoids re-rendering the whole sidebar
  // subtree per frame).
  const asideRef = useRef<HTMLElement>(null);
  const { width, onResizeStart: handleResizeStart } = useResizableWidth({
    storageKey: STORAGE_WIDTH,
    defaultWidth: SIDEBAR_DEFAULT,
    min: SIDEBAR_MIN,
    max: SIDEBAR_MAX,
    edge: "right",
    targets: [asideRef],
  });

  // Runtime state.
  const [missions, setMissions] = useState<MissionSummary[]>([]);
  // Flat list of un-archived direct chats. Running ones first, then
  // stopped/crashed ordered by recency. Refreshed on session/exit and
  // runner/activity events. See docs/impls/archive/0003-direct-chats.md.
  const [directSessions, setDirectSessions] = useState<DirectSessionEntry[]>(
    [],
  );
  const [directSessionActivity, setDirectSessionActivity] = useState<
    Record<string, SessionActivityState | undefined>
  >({});
  const sidebarNavigationHistoryRef = useRef<SidebarNavigationHistory>({
    entries: [],
    index: -1,
  });

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

  // CHAT `+` opens the StartChat modal. State is a single boolean —
  // the modal owns its own field state and runner-list fetch.
  const [creatingChat, setCreatingChat] = useState(false);

  // Identify the currently-open runtime so we can highlight the matching
  // sidebar row. `useMatch` returns null when the URL doesn't match.
  const missionMatch = useMatch("/missions/:id");
  const currentMissionId = missionMatch?.params.id ?? null;
  const chatMatch = useMatch("/chats/:sessionId");
  // Which direct-chat session is currently in view. The chat route
  // encodes the session id in the URL (a runner can host multiple
  // chats — see docs/impls/archive/0003-direct-chats.md), so we match by
  // session id rather than handle.
  const currentChatSessionId = chatMatch?.params.sessionId ?? null;

  const sidebarNavigationEntries = useMemo<SidebarNavigationEntry[]>(
    () => [
      ...missions.map((mission) => ({ to: `/missions/${mission.id}` })),
      ...directSessions.map((session) => ({
        to: `/chats/${session.session_id}`,
        state: { sessionStatus: session.status },
      })),
    ],
    [directSessions, missions],
  );

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

  useEffect(() => {
    const currentKey = sidebarRuntimeKeyForPath(location.pathname);
    if (!currentKey) return;
    const history = sidebarNavigationHistoryRef.current;
    if (history.entries[history.index] === currentKey) return;

    const entries = history.entries.slice(0, history.index + 1);
    if (entries[entries.length - 1] !== currentKey) {
      entries.push(currentKey);
    }
    if (entries.length > SIDEBAR_NAVIGATION_HISTORY_LIMIT) {
      entries.splice(0, entries.length - SIDEBAR_NAVIGATION_HISTORY_LIMIT);
    }
    history.entries = entries;
    history.index = entries.length - 1;
  }, [location.pathname]);

  const navigateSidebarPage = useCallback(
    (direction: SidebarNavigationDirection) => {
      const history = sidebarNavigationHistoryRef.current;
      if (history.entries.length < 2 || history.index === -1) return;

      const delta = direction === "next" ? 1 : -1;
      let nextIndex = history.index + delta;
      while (nextIndex >= 0 && nextIndex < history.entries.length) {
        const entry = sidebarNavigationEntries.find(
          (candidate) => candidate.to === history.entries[nextIndex],
        );
        if (entry) {
          history.index = nextIndex;
          const options = entry.state
            ? {
                replace: location.pathname === entry.to,
                state: entry.state,
              }
            : { replace: location.pathname === entry.to };
          navigate(entry.to, options);
          return;
        }
        nextIndex += delta;
      }
    },
    [location.pathname, navigate, sidebarNavigationEntries],
  );

  // ⌘K / Ctrl+K opens the command palette. ⌘T / Ctrl+T opens the Start
  // Chat modal (browser/terminal convention: ⌘T = new tab/chat, ⌘N =
  // new window). ⌘N is owned by the File → New Window menu accelerator
  // (impl 0018) at the OS level, so it's deliberately absent here to
  // avoid a double-fire. Skip while editing text controls so shortcuts
  // don't hijack form input. xterm's hidden textarea is not an editor
  // field from the app's point of view, so Meta shortcuts still win
  // there; Ctrl shortcuts stay with the PTY/TUI.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      const isXtermInput = !!target?.closest(".xterm");
      if (
        (tag === "input" ||
          tag === "textarea" ||
          tag === "select" ||
          target?.isContentEditable) &&
        !isXtermInput
      ) {
        return;
      }
      if (isXtermInput && !e.metaKey) return;
      if (e.key === "k" || e.key === "K") {
        e.preventDefault();
        e.stopPropagation();
        setPaletteOpen(true);
      } else if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        e.stopPropagation();
        setCreatingChat(true);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const direction = sidebarNavigationDirectionFromKey(e);
      if (!direction) return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      const isXtermInput = !!target?.closest(".xterm");
      if (
        (tag === "input" ||
          tag === "textarea" ||
          tag === "select" ||
          target?.isContentEditable) &&
        !isXtermInput
      ) {
        return;
      }
      if (isXtermInput && !e.metaKey) return;
      e.preventDefault();
      e.stopPropagation();
      navigateSidebarPage(direction);
    };
    const onNavigate = (event: Event) => {
      const direction = (event as CustomEvent<{ direction?: unknown }>).detail
        ?.direction;
      if (isSidebarNavigationDirection(direction)) {
        navigateSidebarPage(direction);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener(SIDEBAR_NAVIGATE_EVENT, onNavigate);
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener(SIDEBAR_NAVIGATE_EVENT, onNavigate);
    };
  }, [navigateSidebarPage]);

  useEffect(() => {
    let unlistenEvents: (() => void) | null = null;
    let unlistenChanged: (() => void) | null = null;
    let cancelled = false;
    void Promise.all([
      listen<AppendedEvent>("event/appended", (msg) => {
        const t = msg.payload.event.type;
        if (
          t === "mission_start" ||
          t === "mission_stopped" ||
          t === "ask_human" ||
          t === "human_question" ||
          t === "human_response" ||
          t === "runner_status"
        ) {
          // ask_human/human_question/human_response refresh the pending-ask
          // count badge; runner_status refreshes live mission activity.
          // Cheap query; fires only on these signal types.
          void refreshMissions();
        }
      }),
      listen("mission/changed", () => {
        void refreshMissions();
      }),
    ]).then(([fnEvents, fnChanged]) => {
      if (cancelled) {
        fnEvents();
        fnChanged();
        return;
      }
      unlistenEvents = fnEvents;
      unlistenChanged = fnChanged;
    });
    return () => {
      cancelled = true;
      unlistenEvents?.();
      unlistenChanged?.();
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

  useEffect(() => {
    const visibleIds = new Set(directSessions.map((s) => s.session_id));
    setDirectSessionActivity((prev) => {
      let changed = false;
      const next: Record<string, SessionActivityState | undefined> = {};
      for (const [id, state] of Object.entries(prev)) {
        if (!visibleIds.has(id)) {
          changed = true;
          continue;
        }
        next[id] = state;
      }
      return changed ? next : prev;
    });
  }, [directSessions]);

  // session/exit fires when a running PTY reaps (live → stopped flip).
  // runner/activity fires on every spawn/reap and is our cue that a
  // new direct chat row may have appeared. Both refresh the same list.
  useEffect(() => {
    let unlistenExit: (() => void) | null = null;
    let unlistenActivity: (() => void) | null = null;
    let unlistenArchived: (() => void) | null = null;
    let unlistenUpdated: (() => void) | null = null;
    let unlistenStatus: (() => void) | null = null;
    let cancelled = false;
    void Promise.all([
      listen<{ session_id: string }>("session/exit", (event) => {
        const sessionId = event.payload.session_id;
        setDirectSessionActivity((prev) => {
          if (prev[sessionId] == null) return prev;
          const next = { ...prev };
          delete next[sessionId];
          return next;
        });
        void refreshDirectSessions();
        // A mission slot exiting flips `any_session_live` from true →
        // false (if it was the last live slot). Without this, the
        // dot stays accent until something else triggers a refresh.
        void refreshMissions();
      }),
      listen("runner/activity", () => {
        void refreshDirectSessions();
        // Same reason in reverse: resuming a slot flips
        // `any_session_live` from false → true. `runner/activity`
        // fires whenever the sessions table changes (spawn/exit),
        // covering both directions cheaply.
        void refreshMissions();
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
      listen<SessionActivityEvent>("session/status", (event) => {
        setDirectSessionActivity((prev) => ({
          ...prev,
          [event.payload.session_id]: event.payload.state,
        }));
      }),
    ]).then(([fnExit, fnActivity, fnArchived, fnUpdated, fnStatus]) => {
      if (cancelled) {
        fnExit();
        fnActivity();
        fnArchived();
        fnUpdated();
        fnStatus();
        return;
      }
      unlistenExit = fnExit;
      unlistenActivity = fnActivity;
      unlistenArchived = fnArchived;
      unlistenUpdated = fnUpdated;
      unlistenStatus = fnStatus;
    });
    return () => {
      cancelled = true;
      unlistenExit?.();
      unlistenActivity?.();
      unlistenArchived?.();
      unlistenUpdated?.();
      unlistenStatus?.();
    };
  }, [refreshDirectSessions, refreshMissions]);

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
          try {
            await api.session.kill(session.session_id);
          } catch (e) {
            // The sidebar row can be stale for a beat after the PTY
            // exits. Continue to archive; the backend still refuses
            // rows that are genuinely running.
            console.warn("sidebar: session_kill before archive failed", e);
          }
        }
        await api.session.archive(session.session_id);
        await refreshDirectSessions();
        if (currentChatSessionId === session.session_id) {
          navigate(session.handle ? `/runners/${session.handle}` : "/runners");
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
      const target = `/chats/${entry.session_id}`;
      navigate(target, {
        state: { sessionStatus: entry.status },
        replace: location.pathname === target,
      });
    },
    [navigate, location.pathname],
  );

  // CHAT `+` button — opens the StartChat modal (GH #104). The modal is
  // a takeover, so we don't pre-expand the SESSION section; the new
  // chat row will be visible on return from the spawned chat URL.
  const handleNewDirectChat = useCallback(() => {
    setCreatingChat(true);
  }, []);

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

  const sidebarVisible = !collapsed || previewOpen;
  const sidebarPreview = collapsed && previewOpen;

  return (
    <>
      <aside
        ref={asideRef}
        onMouseLeave={
          sidebarPreview ? () => onPreviewOpenChange(false) : undefined
        }
        style={{ width: sidebarVisible ? width : 0 }}
        className={`${
          collapsed
            ? "absolute left-0 top-0 z-40"
            : "relative"
        } ${
          sidebarPreview ? "shadow-2xl shadow-black/40" : ""
        } flex h-full shrink-0 select-none flex-col overflow-hidden transition-[width] duration-150 ${
          sidebarVisible ? "border-r border-line bg-sidebar" : "bg-transparent"
        }`}
      >
        {sidebarVisible ? (
          <div className="flex min-h-0 flex-1 flex-col pb-4">
            <div data-tauri-drag-region className="h-8 shrink-0" />

            {/* Brand row — open state only. The drag region extends
                below the traffic-light strip so the header band reads
                as one continuous title bar. */}
            <div
              data-tauri-drag-region
              className="flex shrink-0 items-center gap-2 px-5 pb-5 pt-1"
            >
              <BrandMark />
              <span className="text-base font-semibold tracking-tight text-fg">
                {t("Runner")}
              </span>
            </div>
            {/* WORKSPACE keeps natural height; it doesn't compete
                with the scrollable Mission/Chat region below. */}
            <div className="shrink-0">
              <SectionHeader>{t("WORKSPACE")}</SectionHeader>
              <nav className="flex flex-col gap-0.5 px-3 pb-1">
                <NewChatNavRow onOpen={handleNewDirectChat} />
                {/* Search opens a command-palette modal — matches design
                    `Fkoe8`. Default interaction is click-to-callout, not
                    type-in-place, so this lives as a nav row alongside
                    runner/crew rather than an inline input. Placed
                    first in WORKSPACE because it's the highest-velocity
                    entry point — jumping to any mission / runner /
                    crew without scrolling the lists below. */}
                <SearchNavRow onOpen={() => setPaletteOpen(true)} />
                <NavRow icon={Terminal} to="/runners" label={t("runner")} />
                <NavRow icon={Users} to="/crews" label={t("crew")} />
              </nav>
            </div>

            <div className="h-5 shrink-0" />

            {/* Codex-desktop style: Mission and Chat live in one
                natural scroll column. Sections stack by content
                height; no pane reserves empty space. */}
            <div className="min-h-0 flex-1 overflow-y-auto pb-3">
              <section className="flex flex-col">
                <CollapsibleSectionHeader
                  label={t("MISSION")}
                  open={missionsOpen}
                  onToggle={toggleMissions}
                  onPlus={() => setCreatingMission(true)}
                  plusTitle={t("Start mission")}
                />
                {missionsOpen ? (
                  <div className="flex flex-col gap-0.5 px-3 pt-1">
                    {missions.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        {t("No live missions.")}
                      </p>
                    ) : (
                      missions.map((m) => (
                        <MissionRow
                          key={m.id}
                          mission={m}
                          selected={m.id === currentMissionId}
                          onClick={() => openMission(m.id)}
                          onContextMenu={(anchor) => openMissionMenu(m, anchor)}
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

              <section className="mt-5 flex flex-col">
                <CollapsibleSectionHeader
                  label={t("CHAT")}
                  open={sessionsOpen}
                  onToggle={toggleSessions}
                  onPlus={handleNewDirectChat}
                  plusTitle={t("Start a chat")}
                  plusExpanded={creatingChat}
                />
                {sessionsOpen ? (
                  <div className="flex flex-col gap-0.5 px-3 pt-1">
                    {directSessions.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        {t("No chats yet.")}
                      </p>
                    ) : (
                      directSessions.map((s) => (
                        <SessionRow
                          key={s.session_id}
                          session={s}
                          activity={directSessionActivity[s.session_id]}
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
            </div>

            {/* Settings row — pinned at the bottom of the sidebar
                column. Mirrors Pencil node `IJsUO` (sidebar settings).
                The chevron points toward the action: left collapses
                an open sidebar, right pins a hover-preview sidebar. */}
            <div className="flex shrink-0 items-center gap-2 border-t border-line px-3 pt-2">
              <button
                type="button"
                onClick={() => setSettingsOpen(true)}
                className="flex flex-1 cursor-pointer items-center gap-2.5 rounded border border-transparent px-2.5 py-2 text-left text-fg-2 transition-colors hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg focus:outline-none"
              >
                <SettingsIcon aria-hidden className="h-3.5 w-3.5" />
                <span className="text-[13px]">{t("Settings")}</span>
              </button>
              <button
                type="button"
                onClick={() => {
                  if (sidebarPreview) {
                    onCollapsedChange(false);
                    onPreviewOpenChange(false);
                    return;
                  }
                  onCollapsedChange(true);
                }}
                title={
                  sidebarPreview
                    ? t("Keep sidebar open")
                    : t("Collapse sidebar ({mod}S)", { mod: MOD_LABEL })
                }
                aria-label={
                  sidebarPreview ? t("Keep sidebar open") : t("Collapse sidebar")
                }
                className="flex h-6 w-6 cursor-pointer items-center justify-center rounded border border-transparent text-fg-3 transition-colors hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg focus:outline-none"
              >
                {sidebarPreview ? (
                  <ChevronsRight aria-hidden className="h-3.5 w-3.5" />
                ) : (
                  <ChevronsLeft aria-hidden className="h-3.5 w-3.5" />
                )}
              </button>
            </div>
          </div>
        ) : null}

        {sidebarVisible ? (
          <div
            onPointerDown={handleResizeStart}
            title={t("Drag to resize")}
            className="absolute right-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
          />
        ) : null}
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

      <StartChatModal
        open={creatingChat}
        onClose={() => setCreatingChat(false)}
        onStarted={(spawned) => {
          setCreatingChat(false);
          navigate(`/chats/${spawned.id}`, {
            state: { sessionStatus: "running" },
          });
        }}
      />

      {sessionMenu ? (
        <RowContextMenu
          pinned={sessionMenu.session.pinned}
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
          onOpenInNewWindow={() => {
            void api.window
              .open(`/chats/${sessionMenu.session.session_id}`)
              .catch((e) =>
                console.error("sidebar: open chat in new window failed", e),
              );
            closeSessionMenu();
          }}
          onArchive={() => {
            void archiveSession(sessionMenu.session);
            closeSessionMenu();
          }}
        />
      ) : null}

      {missionMenu ? (
        <RowContextMenu
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
          onOpenInNewWindow={() => {
            void api.window
              .open(`/missions/${missionMenu.mission.id}`)
              .catch((e) =>
                console.error("sidebar: open mission in new window failed", e),
              );
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
        `flex items-center gap-2 rounded border px-2.5 py-1.5 text-sm transition-colors ${
          isActive
            ? "border-sidebar-selected-border bg-sidebar-selected font-semibold text-fg shadow-sm"
            : "border-transparent text-fg-2 hover:bg-sidebar-selected/60 hover:text-fg"
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

function NewChatNavRow({ onOpen }: { onOpen: () => void }) {
  const t = useT();
  return (
    <button
      type="button"
      title={t("New chat")}
      onClick={onOpen}
      className="group flex w-full cursor-pointer items-center gap-2 rounded border border-transparent px-2.5 py-1.5 text-left text-sm text-fg-2 transition-colors hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg focus:outline-none"
    >
      <MessageSquarePlus aria-hidden className="h-3 w-3 text-fg-2" />
      <span className="min-w-0 flex-1 truncate">{t("new chat")}</span>
      <span className="shrink-0 rounded border border-line bg-bg px-1.5 py-px font-mono text-[10px] leading-tight text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 group-focus:opacity-100">
        {`${MOD_LABEL}T`}
      </span>
    </button>
  );
}

/// Search nav row — visually indistinguishable from runner/crew rows
/// but opens the CommandPalette modal instead of routing. The ⌘K
/// keyboard binding (registered above) still works; the shortcut
/// hint appears on hover/focus.
function SearchNavRow({ onOpen }: { onOpen: () => void }) {
  const t = useT();
  return (
    <button
      type="button"
      title={t("Search")}
      onClick={onOpen}
      className="group flex w-full cursor-pointer items-center gap-2 rounded border border-transparent px-2.5 py-1.5 text-left text-sm text-fg-2 transition-colors hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg focus:outline-none"
    >
      <Search aria-hidden className="h-3 w-3 text-fg-2" />
      <span className="min-w-0 flex-1 truncate">{t("search")}</span>
      <span className="shrink-0 rounded border border-line bg-bg px-1.5 py-px font-mono text-[10px] leading-tight text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 group-focus:opacity-100">
        {`${MOD_LABEL}K`}
      </span>
    </button>
  );
}

// ---- collapsible section header ---------------------------------------

function CollapsibleSectionHeader({
  label,
  open,
  onToggle,
  onPlus,
  plusTitle,
  plusExpanded,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  onPlus: () => void;
  plusTitle: string;
  /** When the `+` opens a dialog (modal), pass its open state so the
   *  trigger advertises `aria-haspopup="dialog"` + `aria-expanded`. */
  plusExpanded?: boolean;
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
      </button>
      <button
        type="button"
        onClick={onPlus}
        title={plusTitle}
        aria-label={plusTitle}
        aria-haspopup={plusExpanded === undefined ? undefined : "dialog"}
        aria-expanded={plusExpanded}
        className="cursor-pointer rounded p-1 text-fg-2 transition-colors hover:bg-bg hover:text-fg"
      >
        <Plus aria-hidden className="h-3 w-3" />
      </button>
    </div>
  );
}

// ---- sidebar list rows ------------------------------------------------

function SidebarListRow({
  selected,
  label,
  onClick,
  onContextMenu,
  title,
  mono,
  dim,
  dotClassName,
  pinned,
  renaming,
  renameValue,
  renamePlaceholder,
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
  /** Optional explicit status-dot color for rows with richer live state. */
  dotClassName?: string;
  /** Pinned rows show a Pin icon next to the label. */
  pinned?: boolean;
  /** When true, replaces the label with an inline rename input. */
  renaming?: boolean;
  /** Current editable value. Defaults to `label`. */
  renameValue?: string;
  /** Placeholder shown while the editable value is empty. */
  renamePlaceholder?: string;
  onRenameSubmit?: (next: string) => void;
  onRenameCancel?: () => void;
}) {
  const t = useT();
  if (renaming && onRenameSubmit && onRenameCancel) {
    return (
      <SidebarRowRenameInput
        initial={renameValue ?? label}
        placeholder={renamePlaceholder ?? label}
        title={title}
        mono={mono}
        dim={dim}
        dotClassName={dotClassName}
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
          ? "border-sidebar-selected-border bg-sidebar-selected font-semibold text-fg shadow-sm"
          : "border-transparent text-fg-2 hover:bg-sidebar-selected/60 hover:text-fg"
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
            dotClassName ?? (dim ? "bg-fg-3" : "bg-accent")
          }`}
        />
        {pinned ? (
          <Pin
            aria-hidden
            className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3"
          />
        ) : null}
        <span className={`truncate ${mono ? "font-mono" : ""}`}>{label}</span>
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
          title={t("More actions")}
          aria-label={t("More actions")}
          className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 transition-opacity hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
        >
          <MoreHorizontal aria-hidden className="h-3 w-3" />
        </button>
      ) : null}
    </div>
  );
}

function SidebarRowRenameInput({
  initial,
  placeholder,
  title,
  mono,
  dim,
  dotClassName,
  onSubmit,
  onCancel,
}: {
  initial: string;
  placeholder: string;
  title?: string;
  mono?: boolean;
  dim?: boolean;
  dotClassName?: string;
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
    <div
      className="flex w-full items-center gap-2 rounded border border-sidebar-selected-border bg-sidebar-selected px-2.5 py-1.5 text-xs shadow-sm"
      title={title}
    >
      <span
        className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
          dotClassName ?? (dim ? "bg-fg-3" : "bg-accent")
        }`}
      />
      <input
        ref={inputRef}
        value={draft}
        placeholder={placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onSubmit(draft.trim());
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (draft.trim() === initial.trim()) onCancel();
          else onSubmit(draft.trim());
        }}
        className={`min-w-0 flex-1 bg-transparent text-xs text-fg outline-none placeholder:text-fg-3 ${
          mono ? "font-mono" : ""
        }`}
      />
    </div>
  );
}

// SESSION row: adapter from DirectSessionEntry to the shared sidebar
// row shell. Keeps chat-specific label and rename-null semantics out
// of the generic visual component.
type DirectChatDisplayStatus = SessionActivityState | "stopped" | "crashed";

function directChatDisplayStatus(
  session: DirectSessionEntry,
  activity: SessionActivityState | undefined,
): DirectChatDisplayStatus {
  if (session.status === "stopped" || session.status === "crashed") {
    return session.status;
  }
  return activity ?? "busy";
}

function directChatDotClassName(status: DirectChatDisplayStatus): string {
  switch (status) {
    case "busy":
      return "bg-accent";
    case "idle":
      return "bg-accent/30";
    case "crashed":
      return "bg-danger";
    case "stopped":
      return "bg-fg-3";
  }
}

function missionActivityDotClassName(
  activity: MissionActivityState | null,
): string {
  switch (activity) {
    case "busy":
      return "bg-accent";
    case "idle":
      return "bg-accent/30";
    case null:
      return "bg-fg-3";
  }
}

function MissionRow({
  mission,
  selected,
  renaming,
  onClick,
  onContextMenu,
  onRenameSubmit,
  onRenameCancel,
}: {
  mission: MissionSummary;
  selected: boolean;
  renaming: boolean;
  onClick: () => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  onRenameSubmit: (nextTitle: string) => void;
  onRenameCancel: () => void;
}) {
  const t = useT();
  const activity = mission.any_session_live ? (mission.activity ?? "busy") : null;
  const statusLabel = activity ?? t("paused");
  const tooltip = `${mission.crew_name || t("Mission")} · ${statusLabel}${
    mission.pinned_at ? ` · ${t("pinned")}` : ""
  }`;

  return (
    <SidebarListRow
      selected={selected}
      label={mission.title}
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={tooltip}
      dim={!mission.any_session_live}
      dotClassName={missionActivityDotClassName(activity)}
      pinned={!!mission.pinned_at}
      renaming={renaming}
      onRenameSubmit={onRenameSubmit}
      onRenameCancel={onRenameCancel}
    />
  );
}

function SessionRow({
  session,
  activity,
  selected,
  renaming,
  onClick,
  onContextMenu,
  onRenameSubmit,
  onRenameCancel,
}: {
  session: DirectSessionEntry;
  activity: SessionActivityState | undefined;
  selected: boolean;
  renaming: boolean;
  onClick: () => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  onRenameSubmit: (nextTitle: string | null) => void;
  onRenameCancel: () => void;
}) {
  const t = useT();
  const defaultLabel = session.handle
    ? `@${session.handle} · ${formatStartedAt(session, t)}`
    : `${session.display_name} · ${formatStartedAt(session, t)}`;
  const label = session.title ?? defaultLabel;
  const dim = session.status !== "running";
  const displayStatus = directChatDisplayStatus(session, activity);
  const tooltip = `${session.handle ? `@${session.handle}` : session.display_name} · ${displayStatus}${
    session.status !== "running" && session.resumable ? ` · ${t("resumable")}` : ""
  }${session.pinned ? ` · ${t("pinned")}` : ""}`;

  return (
    <SidebarListRow
      selected={selected}
      label={label}
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={tooltip}
      mono={!!session.handle}
      dim={dim}
      dotClassName={directChatDotClassName(displayStatus)}
      pinned={session.pinned}
      renaming={renaming}
      renameValue={session.title ?? ""}
      renamePlaceholder={defaultLabel}
      onRenameSubmit={(next) => {
        onRenameSubmit(next.length === 0 ? null : next);
      }}
      onRenameCancel={onRenameCancel}
    />
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

// Floating Pin / Rename / Archive menu shared by mission rows and
// direct chat rows. Layout matches Pencil node `EWpGa`: 160px wide,
// 6px padding, lucide icons, dark surface with a subtle drop shadow.
// Closes on outside click, Escape, or any action firing.
function RowContextMenu({
  pinned,
  anchorX,
  anchorY,
  onClose,
  onPin,
  onRename,
  onOpenInNewWindow,
  onArchive,
}: {
  pinned: boolean;
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onOpenInNewWindow: () => void;
  onArchive: () => void;
}) {
  const t = useT();
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
        label={pinned ? t("Unpin") : t("Pin")}
        onClick={onPin}
      />
      <ContextMenuItem icon={SquarePen} label={t("Rename")} onClick={onRename} />
      {/* Multi-window is macOS-only for now: WebView2 secondary windows load
          blank on Windows/Linux, so hide the action there. */}
      {isMac && (
        <ContextMenuItem
          icon={AppWindow}
          label={t("Open in New Window")}
          onClick={onOpenInNewWindow}
        />
      )}
      <ContextMenuItem
        icon={Archive}
        label={t("Archive")}
        onClick={onArchive}
        danger
      />
    </div>
  );
}

// Cheap relative-ish label for sessions that have no user-set title.
// Prefers the started_at column; falls back to stopped_at if both are
// set (older rows stay sortable). Months are short to keep the row narrow.
function formatStartedAt(s: DirectSessionEntry, t: TFn): string {
  const ts = s.started_at ?? s.stopped_at;
  if (!ts) return t("session");
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return t("session");
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
  // Brand-mark tint: when on (default), the chevron picks up the active
  // theme's `var(--color-accent)` via `text-accent`; when off, it pins
  // to the Carbon green `#00FF9C` so the in-sidebar mark stays aligned
  // with the bundled `.icns` icon on Dock / Cmd+Tab / notifications.
  // The polylines below use `stroke="currentColor"`, so this single
  // `text-…` / `style.color` selection cascades through.
  const [tint, setTint] = useState<boolean>(() => readBrandTint());
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_APP_BRAND_TINT) return;
      setTint(readBrandTint());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  return (
    <svg
      width="32"
      height="32"
      viewBox="0 0 32 32"
      aria-hidden
      className={`shrink-0 ${tint ? "text-accent" : ""}`}
      style={tint ? undefined : { color: BRAND_MARK_PINNED_COLOR }}
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
        stroke="currentColor"
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
