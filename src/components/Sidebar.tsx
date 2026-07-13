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
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type DragEvent,
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
  Folder,
  FolderPlus,
  MessageSquarePlus,
  MoreHorizontal,
  Pin,
  PinOff,
  Plus,
  Search,
  Settings as SettingsIcon,
  SquarePen,
  Terminal,
  Trash2,
  Users,
} from "lucide-react";

import { api, type DirectSessionEntry } from "../lib/api";
import {
  markArchivingMission,
  markArchivingSession,
  unmarkArchivingMission,
  unmarkArchivingSession,
} from "../lib/archivingState";
import {
  groupPinTargets,
  pinnedSessionIds,
  shouldInheritPinOnAdd,
} from "../lib/groupPinning";
import {
  isChatTabDropIndexAllowed,
  orderedChatTabIdsAfterDrop,
} from "../lib/chatTabs";
import {
  rollupAttentionState,
  tabAttentionState,
  useDirectSessionActivity,
  type ChatAttentionState,
} from "../lib/chatAttention";
import {
  activatePaneLayoutForSession,
  assignSessionToPane,
  createChatFolder,
  findLeaf,
  focusPane,
  getPaneLayout,
  leafForSession,
  newChatTargetPane,
  removeArchivedSessionFromLayout,
  hydratePaneLayoutsFromDb,
  moveSessionTabToFolder,
  moveTabToFolder,
  reorderTab,
  setGroupNameForSession,
  useFolders,
  usePaneLayouts,
  visibleSessionIds,
  type PaneLayout,
} from "../lib/paneLayout";
import {
  CHAT_TAB_DRAG_TYPE,
  ChatAttentionIndicator,
  ChatTabGroup,
} from "./ChatTabGroup";
import { PanelToggleGlyph } from "./PanelToggleGlyph";
import { PopoverMenu } from "./ui/PopoverMenu";
import { useResizableWidth } from "../hooks/useResizableWidth";
import {
  BRAND_MARK_PINNED_COLOR,
  readBrandTint,
  STORAGE_APP_BRAND_TINT,
} from "../lib/settings";
import { reportSubjectsNow } from "../lib/windowFocus";
import type {
  AppendedEvent,
  MissionActivityState,
  MissionSummary,
  SessionActivityState,
} from "../lib/types";
import { StartMissionModal } from "./StartMissionModal";
import { StartChatModal } from "./StartChatModal";
import { CommandPalette } from "./CommandPalette";
import { UpdatePromptCard } from "./UpdatePromptCard";
import { ConfirmDialog } from "./settings/ConfirmDialog";

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

// Cmd+Shift+[ / Cmd+Shift+] — page navigation moved to the shifted pair
// (the tab-switch idiom in iTerm2/Safari/VS Code) so plain Cmd+[ / Cmd+]
// can cycle split-pane focus in the chat surface (impl 0020), matching
// iTerm2's pane/tab split. Shifted brackets arrive as "{" / "}" on US
// layouts, so match on `code` first with the shifted keys as fallback.
// Documented in src/lib/keymap.ts (page-navigation).
function sidebarNavigationDirectionFromKey(
  e: KeyboardEvent,
): SidebarNavigationDirection | null {
  if (!(e.metaKey || e.ctrlKey)) return null;
  if (e.altKey || !e.shiftKey) return null;
  if (e.code === "BracketLeft" || e.key === "[" || e.key === "{") {
    return "previous";
  }
  if (e.code === "BracketRight" || e.key === "]" || e.key === "}") {
    return "next";
  }
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
  // Collapsed/expanded state lives in AppShell so the global Cmd+S
  // shortcut can toggle it too. The `width` resize state stays local —
  // it's preserved across collapse/expand cycles so users get their last
  // full width back when they re-open.
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  previewOpen: boolean;
  onPreviewOpenChange: (open: boolean) => void;
}

export function Sidebar({
  collapsed,
  onCollapsedChange,
  previewOpen,
  onPreviewOpenChange,
}: SidebarProps) {
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
  const directSessionActivity = useDirectSessionActivity();
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
  const [chatTabMenu, setChatTabMenu] = useState<{
    layout: PaneLayout;
    members: DirectSessionEntry[];
    x: number;
    y: number;
  } | null>(null);
  const [folderMenu, setFolderMenu] = useState<{
    id: string;
    name: string;
    x: number;
    y: number;
  } | null>(null);
  const [folderDeleteConfirm, setFolderDeleteConfirm] = useState<{
    id: string;
    name: string;
    count: number;
    currentWasDeleted: boolean;
  } | null>(null);
  const [deletingFolder, setDeletingFolder] = useState(false);
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
  // CHAT creation state. The `+` and empty-space context menus can start a
  // chat or insert a focused inline folder-name row.
  const [creatingChat, setCreatingChat] = useState(false);
  const [newChatFolderId, setNewChatFolderId] = useState<string | null>(null);
  const [chatAddMenuOpen, setChatAddMenuOpen] = useState(false);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [chatCreateMenu, setChatCreateMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const [dragOverFolderId, setDragOverFolderId] = useState<string | null>(null);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [tabDropTarget, setTabDropTarget] = useState<{
    folderId: string | null;
    index: number;
    markerKey: string;
  } | null>(null);

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

  // Durable tab/folder state. Every layout renders as one sidebar row;
  // member panes remain visible only on the chat surface.
  const paneLayouts = usePaneLayouts();
  const folders = useFolders();

  const tabItems = useMemo(() => {
    const byId = new Map(
      directSessions.map((session) => [session.session_id, session]),
    );
    return paneLayouts
      .map((layout) => {
        const members = visibleSessionIds(layout.root)
          .map((id) => byId.get(id))
          .filter(
            (session): session is DirectSessionEntry => session !== undefined,
          );
        return {
          layout,
          members,
          attention: tabAttentionState(
            members,
            directSessionActivity,
            layout.lastCompletedAt,
            layout.lastViewedAt,
          ),
        };
      })
      .filter((item) => item.members.length > 0)
      .sort(
        (a, b) =>
          Number(b.members.every((member) => member.pinned)) -
          Number(a.members.every((member) => member.pinned)),
      );
  }, [directSessionActivity, directSessions, paneLayouts]);
  const chatAttention = useMemo(
    () => rollupAttentionState(tabItems.map((item) => item.attention)),
    [tabItems],
  );
  const orderedChatRows = useMemo(
    () => tabItems.map((item) => item.members[0]),
    [tabItems],
  );

  const sidebarNavigationEntries = useMemo<SidebarNavigationEntry[]>(
    () => [
      ...missions.map((mission) => ({ to: `/missions/${mission.id}` })),
      ...orderedChatRows.map((session) => ({
        to: `/chats/${session.session_id}`,
        state: { sessionStatus: session.status },
      })),
    ],
    [orderedChatRows, missions],
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
  // Documented in src/lib/keymap.ts (command-palette, new-chat).
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
      await hydratePaneLayoutsFromDb();
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
  const openChatTabMenu = useCallback(
    (
      layout: PaneLayout,
      members: DirectSessionEntry[],
      anchor: { x: number; y: number },
    ) => {
      setMissionMenu(null);
      setFolderMenu(null);
      setChatTabMenu({ layout, members, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeChatTabMenu = useCallback(() => setChatTabMenu(null), []);

  const openMissionMenu = useCallback(
    (mission: MissionSummary, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setFolderMenu(null);
      setMissionMenu({ mission, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeFolderMenu = useCallback(() => setFolderMenu(null), []);
  const openFolderMenu = useCallback(
    (folder: { id: string; name: string }, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setMissionMenu(null);
      setFolderMenu({ ...folder, x: anchor.x, y: anchor.y });
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
    [missions, refreshMissions, setRenamingMissionId],
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
        // Archived rows can't stay on screen — empty out any split pane
        // that was showing this chat (no-op when not split / not visible).
        removeArchivedSessionFromLayout(session.session_id);
        await refreshDirectSessions();
        if (currentChatSessionId === session.session_id) {
          // Prefer handing the URL to a surviving group member over
          // leaving the chat surface (mirrors RunnerChat.archiveSession).
          const next = visibleSessionIds(
            getPaneLayout(currentChatSessionId).root,
          ).find(
            (id) => id !== session.session_id,
          );
          if (next) {
            const nextLeaf = leafForSession(
              getPaneLayout(currentChatSessionId).root,
              next,
            );
            if (nextLeaf) focusPane(nextLeaf.id);
            navigate(`/chats/${next}`, { replace: true });
          } else {
            navigate(
              session.handle ? `/runners/${session.handle}` : "/runners",
            );
          }
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

  const setChatTabPin = useCallback(
    async (members: DirectSessionEntry[], nextPinned: boolean) => {
      const memberIds = members.map((member) => member.session_id);
      const firstId = memberIds[0];
      if (!firstId) return;
      const targets = groupPinTargets(
        firstId,
        memberIds,
        pinnedSessionIds(directSessions),
        nextPinned,
      );
      try {
        await Promise.all(targets.map((id) => api.session.pin(id, nextPinned)));
        await refreshDirectSessions();
      } catch (e) {
        console.error("sidebar: chat tab session_pin failed", e);
      }
    },
    [directSessions, refreshDirectSessions],
  );

  const renameChatTab = useCallback((members: DirectSessionEntry[]) => {
    const first = members[0];
    if (!first) return;
    const layout = getPaneLayout(first.session_id);
    const proposed = window.prompt(
      "Rename tab (blank = derive from chats)",
      layout.name ?? "",
    );
    if (proposed === null) return;
    setGroupNameForSession(first.session_id, proposed);
  }, []);

  const beginFolderCreate = useCallback(() => {
    setChatAddMenuOpen(false);
    setChatCreateMenu(null);
    setSessionsOpen(true);
    setStoredFlag(STORAGE_SESSION_OPEN, true);
    setCreatingFolder(true);
  }, []);

  const submitFolderCreate = useCallback(async (name: string) => {
    try {
      await createChatFolder(name);
      setCreatingFolder(false);
    } catch (e) {
      console.error("sidebar: folder_create failed", e);
      throw e;
    }
  }, []);

  const openChatCreateMenu = useCallback(
    (anchor: { x: number; y: number }) => {
      setChatAddMenuOpen(false);
      setChatTabMenu(null);
      setFolderMenu(null);
      setMissionMenu(null);
      setChatCreateMenu(anchor);
    },
    [],
  );

  const renameFolder = useCallback(async (id: string, currentName: string) => {
    const name = window.prompt("Rename folder", currentName);
    if (!name?.trim() || name.trim() === currentName) return;
    try {
      await api.folder.rename(id, name.trim());
      await hydratePaneLayoutsFromDb();
    } catch (e) {
      console.error("sidebar: folder_rename failed", e);
    }
  }, []);

  const toggleFolder = useCallback(async (id: string, collapsed: boolean) => {
    try {
      await api.folder.setCollapsed(id, !collapsed);
      await hydratePaneLayoutsFromDb();
    } catch (e) {
      console.error("sidebar: folder_set_collapsed failed", e);
    }
  }, []);

  const requestFolderDelete = useCallback(
    (id: string, name: string) => {
      const memberTabs = tabItems.filter((item) => item.layout.folderId === id);
      const count = paneLayouts.filter((layout) => layout.folderId === id).length;
      const currentWasDeleted = memberTabs.some((item) =>
        item.members.some((member) => member.session_id === currentChatSessionId),
      );
      setFolderDeleteConfirm({
        id,
        name,
        count,
        currentWasDeleted,
      });
    },
    [currentChatSessionId, paneLayouts, tabItems],
  );

  const deleteFolder = useCallback(
    async (confirm: NonNullable<typeof folderDeleteConfirm>) => {
      setDeletingFolder(true);
      try {
        await api.folder.delete(confirm.id);
        await Promise.all([refreshDirectSessions(), hydratePaneLayoutsFromDb()]);
        setFolderDeleteConfirm(null);
        if (confirm.currentWasDeleted) navigate("/runners");
      } catch (e) {
        console.error("sidebar: folder_delete failed", e);
      } finally {
        setDeletingFolder(false);
      }
    },
    [navigate, refreshDirectSessions],
  );

  const openChatTabInNewWindow = useCallback((members: DirectSessionEntry[]) => {
    const first = members[0];
    if (!first) return;
    const layout = getPaneLayout(first.session_id);
    const focusedSessionId =
      findLeaf(layout.root, layout.focusedPaneId)?.sessionId ?? null;
    const target =
      members.find((member) => member.session_id === focusedSessionId) ?? first;
    void api.window.open(`/chats/${target.session_id}`).catch((e) =>
      console.error("sidebar: open chat tab in new window failed", e),
    );
  }, []);

  const archiveChatTab = useCallback(
    async (members: DirectSessionEntry[]) => {
      const ordered = [...members].sort((a, b) => {
        if (a.session_id === currentChatSessionId) return 1;
        if (b.session_id === currentChatSessionId) return -1;
        return 0;
      });
      for (const member of ordered) {
        await archiveSession(member);
      }
    },
    [archiveSession, currentChatSessionId],
  );

  // A tab-row click activates that tab and opens its focused pane's chat.
  const activateTabChat = useCallback(
    (
      tabId: string,
      members: DirectSessionEntry[],
      entry: DirectSessionEntry,
    ) => {
      void api.tab
        .markViewed(
          tabId,
          members.map((member) => member.session_id),
        )
        .catch((error) =>
          console.error("sidebar: tab_mark_viewed failed", error),
        );
      const entryLayout = activatePaneLayoutForSession(entry.session_id);
      const entryLeaf = leafForSession(entryLayout.root, entry.session_id);
      if (entryLeaf) focusPane(entryLeaf.id);
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
    setChatAddMenuOpen(false);
    setChatCreateMenu(null);
    setCreatingFolder(false);
    setNewChatFolderId(null);
    setCreatingChat(true);
  }, []);

  const handleNewFolderChat = useCallback((folderId: string) => {
    setChatAddMenuOpen(false);
    setChatCreateMenu(null);
    setCreatingFolder(false);
    setNewChatFolderId(folderId);
    setCreatingChat(true);
  }, []);

  const clearTabDrag = useCallback(() => {
    setDraggedTabId(null);
    setDragOverFolderId(null);
    setTabDropTarget(null);
  }, []);

  const commitTabDrop = useCallback(
    async (tabId: string, folderId: string | null, requestedIndex: number) => {
      const dragged = tabItems.find((item) => item.layout.id === tabId);
      if (!dragged) {
        clearTabDrag();
        return;
      }
      const pinned = dragged.members.every((member) => member.pinned);
      const targetItems = tabItems.filter(
        (item) =>
          item.layout.folderId === folderId && item.layout.id !== tabId,
      );
      const orderedIds = orderedChatTabIdsAfterDrop(
        targetItems.map((item) => ({
          id: item.layout.id,
          pinned: item.members.every((member) => member.pinned),
        })),
        tabId,
        pinned,
        requestedIndex,
      );
      clearTabDrag();
      try {
        await reorderTab(tabId, folderId, orderedIds);
      } catch (error) {
        console.error("sidebar: reorder tab failed", error);
      }
    },
    [clearTabDrag, tabItems],
  );

  const dropTabIntoFolder = useCallback(
    (tabId: string, folderId: string) => {
      setDragOverFolderId(null);
      void commitTabDrop(tabId, folderId, Number.MAX_SAFE_INTEGER);
    },
    [commitTabDrop],
  );

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

  const renderTabItem = (item: (typeof tabItems)[number]) => {
    const active = item.members.some(
      (member) => member.session_id === currentChatSessionId,
    );
    return (
      <ChatTabGroup
        key={item.layout.id}
        layout={item.layout}
        members={item.members}
        active={active}
        attention={item.attention}
        onActivate={(entry) =>
          activateTabChat(item.layout.id, item.members, entry)
        }
        onContextMenu={(anchor) =>
          openChatTabMenu(item.layout, item.members, anchor)
        }
        onDragStart={(tabId) => {
          setDraggedTabId(tabId);
          setDragOverFolderId(null);
          setTabDropTarget(null);
        }}
        onDragEnd={clearTabDrag}
        dragging={draggedTabId === item.layout.id}
      />
    );
  };

  const renderTabDropDivider = (
    folderId: string | null,
    items: typeof tabItems,
    originalIndex: number,
    key: string,
  ) => {
    if (!draggedTabId) return null;
    const dragged = tabItems.find((item) => item.layout.id === draggedTabId);
    if (!dragged) return null;
    const index = items
      .slice(0, originalIndex)
      .filter((item) => item.layout.id !== draggedTabId).length;
    if (
      !isChatTabDropIndexAllowed(
        items.map((item) => ({
          id: item.layout.id,
          pinned: item.members.every((member) => member.pinned),
        })),
        dragged.layout.id,
        dragged.members.every((member) => member.pinned),
        index,
      )
    ) {
      return null;
    }
    return (
      <TabDropDivider
        key={key}
        active={
          tabDropTarget?.folderId === folderId &&
          tabDropTarget.index === index &&
          tabDropTarget.markerKey === key
        }
        onDragOver={(event) => {
          event.preventDefault();
          event.stopPropagation();
          event.dataTransfer.dropEffect = "move";
          setDragOverFolderId(null);
          setTabDropTarget({ folderId, index, markerKey: key });
        }}
        onDrop={(event) => {
          event.preventDefault();
          event.stopPropagation();
          const tabId = event.dataTransfer.getData(CHAT_TAB_DRAG_TYPE);
          if (tabId) void commitTabDrop(tabId, folderId, index);
          else clearTabDrag();
        }}
      />
    );
  };

  const renderTabItems = (
    items: typeof tabItems,
    folderId: string | null,
  ) => (
    <>
      {items.map((item, originalIndex) => {
        const dropIndex = (after: boolean) =>
          items
            .slice(0, originalIndex + (after ? 1 : 0))
            .filter((candidate) => candidate.layout.id !== draggedTabId)
            .length;
        const canDropAt = (index: number) => {
          const dragged = tabItems.find(
            (candidate) => candidate.layout.id === draggedTabId,
          );
          if (!dragged) return false;
          return isChatTabDropIndexAllowed(
            items.map((candidate) => ({
              id: candidate.layout.id,
              pinned: candidate.members.every((member) => member.pinned),
            })),
            dragged.layout.id,
            dragged.members.every((member) => member.pinned),
            index,
          );
        };
        const pointerIndex = (event: DragEvent<HTMLDivElement>) => {
          const rect = event.currentTarget.getBoundingClientRect();
          return dropIndex(event.clientY >= rect.top + rect.height / 2);
        };
        return (
          <Fragment key={item.layout.id}>
            {renderTabDropDivider(
              folderId,
              items,
              originalIndex,
              `before-${item.layout.id}`,
            )}
            <div
              onDragOver={(event) => {
                if (
                  !Array.from(event.dataTransfer.types).includes(
                    CHAT_TAB_DRAG_TYPE,
                  )
                ) {
                  return;
                }
                const index = pointerIndex(event);
                if (!canDropAt(index)) {
                  event.preventDefault();
                  event.stopPropagation();
                  event.dataTransfer.dropEffect = "none";
                  setTabDropTarget(null);
                  return;
                }
                event.preventDefault();
                event.stopPropagation();
                event.dataTransfer.dropEffect = "move";
                setDragOverFolderId(null);
                const after =
                  event.clientY >=
                  event.currentTarget.getBoundingClientRect().top +
                    event.currentTarget.getBoundingClientRect().height / 2;
                const markerKey = after
                  ? originalIndex + 1 < items.length
                    ? `before-${items[originalIndex + 1].layout.id}`
                    : `after-${folderId ?? "ungrouped"}`
                  : `before-${item.layout.id}`;
                setTabDropTarget({ folderId, index, markerKey });
              }}
              onDrop={(event) => {
                const index = pointerIndex(event);
                if (!canDropAt(index)) {
                  event.preventDefault();
                  event.stopPropagation();
                  clearTabDrag();
                  return;
                }
                event.preventDefault();
                event.stopPropagation();
                const tabId = event.dataTransfer.getData(CHAT_TAB_DRAG_TYPE);
                if (tabId) void commitTabDrop(tabId, folderId, index);
                else clearTabDrag();
              }}
            >
              {renderTabItem(item)}
            </div>
          </Fragment>
        );
      })}
      {renderTabDropDivider(
        folderId,
        items,
        items.length,
        `after-${folderId ?? "ungrouped"}`,
      )}
    </>
  );

  const sidebarVisible = !collapsed || previewOpen;
  const sidebarPreview = collapsed && previewOpen;
  // Keep the panel mounted through the collapse width-animation so it
  // slides out under the overflow clip instead of blinking away — the
  // mirror of the expand animation. `contentMounted` lingers true after
  // `sidebarVisible` flips false; a `width` transitionend then clears it.
  const [contentMounted, setContentMounted] = useState(sidebarVisible);
  useEffect(() => {
    if (sidebarVisible) setContentMounted(true);
  }, [sidebarVisible]);
  const showPanel = sidebarVisible || contentMounted;
  // The collapsed hover-peek renders as a raised, docked overlay with a
  // rounded right edge (Arc-ish), fading + sliding in on hover. `peeking`
  // holds that overlay treatment through the fade-out so it stays absolute
  // (never shoving the main content); pinning it open clears it, as does the
  // width transitionend once fully closed.
  const [peeking, setPeeking] = useState(false);
  useEffect(() => {
    if (previewOpen) setPeeking(true);
    else if (!collapsed) setPeeking(false);
  }, [previewOpen, collapsed]);
  const peekOverlay = collapsed && (previewOpen || peeking);

  return (
    <>
      <aside
        ref={asideRef}
        onMouseLeave={
          sidebarPreview ? () => onPreviewOpenChange(false) : undefined
        }
        onTransitionEnd={(e) => {
          if (e.propertyName === "width" && !sidebarVisible) {
            setContentMounted(false);
            setPeeking(false);
          }
        }}
        style={{
          width: sidebarVisible ? width : 0,
          opacity: sidebarVisible ? 1 : 0,
        }}
        className={`flex shrink-0 select-none flex-col overflow-hidden transition-[width,opacity] duration-150 ${
          peekOverlay
            ? // Collapsed peek: a raised, docked overlay with a rounded right
              // edge. Kept absolute (via `peeking`) through the fade-out so it
              // never shoves the main content on the way in or out.
              "absolute inset-y-0 left-0 z-40 rounded-r-xl border-r border-line bg-sidebar shadow-2xl shadow-black/40"
            : // Docked: in-flow so the width animation pushes/pulls the main
              // content symmetrically on both expand and collapse.
              `relative h-full ${
                showPanel ? "border-r border-line bg-sidebar" : "bg-transparent"
              }`
        }`}
      >
        {showPanel ? (
          <div className="flex min-h-0 flex-1 flex-col pb-4">
            <div data-tauri-drag-region className="h-8 shrink-0" />

            {/* Brand row — open state only. The drag region extends
                below the traffic-light strip so the header band reads
                as one continuous title bar. The trailing magnifier
                opens the command palette (Codex-style: title left,
                search icon right) — replaced the old WORKSPACE search
                nav row to give the lists below one more row of space.
                A child button inside the drag region is safe: Tauri
                starts a drag only when the mousedown target itself
                carries the attribute. */}
            <div
              data-tauri-drag-region
              className="flex shrink-0 items-center gap-2 px-5 pb-5 pt-1"
            >
              <BrandMark />
              <span className="text-base font-semibold tracking-tight text-fg">
                Runner
              </span>
              <button
                type="button"
                onClick={() => setPaletteOpen(true)}
                title="Search (⌘K)"
                aria-label="Search"
                className="ml-auto flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded border border-transparent text-fg-2 transition-colors hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg focus:border-sidebar-selected-border focus:bg-sidebar-selected/40 focus:text-fg focus:outline-none"
              >
                <Search aria-hidden className="h-3.5 w-3.5" />
              </button>
            </div>
            {/* WORKSPACE keeps natural height; it doesn't compete
                with the scrollable Mission/Chat region below. */}
            <div className="shrink-0">
              <SectionHeader>WORKSPACE</SectionHeader>
              <nav className="flex flex-col gap-0.5 px-3 pb-1">
                <NavRow icon={Terminal} to="/runners" label="runner" />
                <NavRow icon={Users} to="/crews" label="crew" />
              </nav>
            </div>

            <div className="h-5 shrink-0" />

            {/* Mission and Chat get INDEPENDENT scroll regions so a long
                chat list can't scroll the mission tray out of view. The
                mission tray is capped and self-scrolls; Chat fills and
                scrolls the rest. */}
            <div className="flex min-h-0 flex-1 flex-col pb-3">
              <section className="flex shrink-0 flex-col">
                <CollapsibleSectionHeader
                  label="MISSION"
                  open={missionsOpen}
                  onToggle={toggleMissions}
                  onPlus={() => setCreatingMission(true)}
                  plusTitle="Start mission"
                />
                {missionsOpen ? (
                  <div className="flex max-h-[38vh] flex-col gap-0.5 overflow-y-auto px-3 pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
                    {missions.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No live missions.
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

              <section className="mt-5 flex min-h-0 flex-1 flex-col">
                <CollapsibleSectionHeader
                  label="CHAT"
                  open={sessionsOpen}
                  attention={sessionsOpen ? null : chatAttention}
                  onToggle={toggleSessions}
                  onPlus={() => {
                    setChatCreateMenu(null);
                    setChatAddMenuOpen((open) => !open);
                  }}
                  plusTitle="Add chat or folder"
                  plusExpanded={chatAddMenuOpen}
                  plusPopup="menu"
                  onPlusMenuClose={() => setChatAddMenuOpen(false)}
                  plusMenu={
                    <div className="flex flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]">
                      <button
                        type="button"
                        onClick={() => {
                          setChatAddMenuOpen(false);
                          handleNewDirectChat();
                        }}
                        className="flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] text-fg hover:bg-line"
                      >
                        <MessageSquarePlus aria-hidden className="h-3.5 w-3.5" />
                        New chat
                      </button>
                      <button
                        type="button"
                        onClick={beginFolderCreate}
                        className="flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] text-fg hover:bg-line"
                      >
                        <FolderPlus aria-hidden className="h-3.5 w-3.5" />
                        New folder
                      </button>
                    </div>
                  }
                />
                {sessionsOpen ? (
                  <div
                    onContextMenu={(event) => {
                      if (event.defaultPrevented) return;
                      event.preventDefault();
                      openChatCreateMenu({
                        x: event.clientX,
                        y: event.clientY,
                      });
                    }}
                    className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-3 pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
                  >
                    {creatingFolder ? (
                      <NewFolderRow
                        onSubmit={submitFolderCreate}
                        onCancel={() => setCreatingFolder(false)}
                      />
                    ) : null}
                    {folders.length === 0 &&
                    tabItems.length === 0 &&
                    !creatingFolder ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No chats yet.
                      </p>
                    ) : (
                      <>
                        {folders.map((folder) => {
                          const items = tabItems.filter(
                            (item) => item.layout.folderId === folder.id,
                          );
                          const folderAttention = rollupAttentionState(
                            items.map((item) => item.attention),
                          );
                          return (
                            <div
                              key={folder.id}
                              onDragOver={(event) => {
                                if (
                                  !Array.from(event.dataTransfer.types).includes(
                                    CHAT_TAB_DRAG_TYPE,
                                  )
                                ) {
                                  return;
                                }
                                event.preventDefault();
                                event.dataTransfer.dropEffect = "move";
                                setDragOverFolderId(folder.id);
                                setTabDropTarget(null);
                              }}
                              onDragLeave={(event) => {
                                const next = event.relatedTarget;
                                if (
                                  !(next instanceof Node) ||
                                  !event.currentTarget.contains(next)
                                ) {
                                  setDragOverFolderId(null);
                                }
                              }}
                              onDrop={(event) => {
                                event.preventDefault();
                                const tabId = event.dataTransfer.getData(
                                  CHAT_TAB_DRAG_TYPE,
                                );
                                if (tabId) {
                                  void dropTabIntoFolder(tabId, folder.id);
                                } else {
                                  setDragOverFolderId(null);
                                }
                              }}
                              className="flex flex-col gap-0.5"
                            >
                              <div
                                onContextMenu={(event) => {
                                  event.preventDefault();
                                  openFolderMenu(folder, {
                                    x: event.clientX,
                                    y: event.clientY,
                                  });
                                }}
                                className={`group flex items-center gap-1.5 rounded border px-2.5 py-1.5 text-xs transition-colors ${
                                  dragOverFolderId === folder.id
                                    ? "border-accent bg-accent/10 text-fg"
                                    : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
                                }`}
                              >
                                <button
                                  type="button"
                                  onClick={() =>
                                    void toggleFolder(folder.id, folder.collapsed)
                                  }
                                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                                >
                                  {folder.collapsed ? (
                                    <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
                                  ) : (
                                    <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
                                  )}
                                  <Folder aria-hidden className="h-3 w-3 shrink-0" />
                                  <span className="min-w-0 flex-1 truncate font-medium">
                                    {folder.name}
                                  </span>
                                  <ChatAttentionIndicator
                                    state={
                                      folder.collapsed ? folderAttention : null
                                    }
                                  />
                                  <span className="text-[10px] text-fg-3">
                                    {items.length}
                                  </span>
                                </button>
                                <button
                                  type="button"
                                  onClick={() => {
                                    if (folder.collapsed) {
                                      void toggleFolder(folder.id, true);
                                    }
                                    handleNewFolderChat(folder.id);
                                  }}
                                  className="cursor-pointer rounded p-0.5 text-fg-3 hover:bg-raised hover:text-fg"
                                  aria-label={`New chat in ${folder.name}`}
                                  title="New chat in folder"
                                >
                                  <Plus aria-hidden className="h-3 w-3" />
                                </button>
                                <button
                                  type="button"
                                  onClick={(event) =>
                                    openFolderMenu(folder, {
                                      x: event.clientX,
                                      y: event.clientY,
                                    })
                                  }
                                  className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
                                  aria-label="Folder actions"
                                  title="Folder actions"
                                >
                                  <MoreHorizontal aria-hidden className="h-3 w-3" />
                                </button>
                              </div>
                              {folder.collapsed ? null : (
                                <div className="ml-3 flex flex-col gap-0.5 border-l border-line pl-2">
                                  {renderTabItems(items, folder.id)}
                                  {items.length === 0 ? (
                                    <p className="px-2.5 py-1.5 text-xs text-fg-3">
                                      Empty folder
                                    </p>
                                  ) : null}
                                </div>
                              )}
                            </div>
                          );
                        })}
                        {renderTabItems(
                          tabItems.filter(
                            (item) => item.layout.folderId === null,
                          ),
                          null,
                        )}
                      </>
                    )}
                  </div>
                ) : null}
              </section>
            </div>

            {/* Update prompt card — floats directly above the Settings
                row when an update is ready to install (impl 0025).
                Renders null otherwise. */}
            <UpdatePromptCard />

            {/* Settings row — pinned at the bottom of the sidebar
                column. Mirrors Pencil node `IJsUO` (sidebar settings).
                Navigates to the full-page settings route (impl 0025),
                threading the current location through state so "Back
                to app" can return here. The trailing button collapses
                the sidebar (or, in a hover-preview, pins it open) via
                the #246 panel glyph. */}
            <div className="flex shrink-0 items-center gap-2 border-t border-sidebar-selected-border px-3 pt-2">
              <button
                type="button"
                onClick={() =>
                  navigate("/settings", {
                    state: { from: location.pathname },
                  })
                }
                className="flex flex-1 cursor-pointer items-center gap-2.5 rounded border border-transparent px-2.5 py-2 text-left text-fg-2 transition-colors hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg focus:border-sidebar-selected-border focus:bg-sidebar-selected/40 focus:text-fg focus:outline-none"
              >
                <SettingsIcon aria-hidden className="h-3.5 w-3.5" />
                <span className="text-[13px]">Settings</span>
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
                  sidebarPreview ? "Keep sidebar open" : "Collapse sidebar (⌘S)"
                }
                aria-label={
                  sidebarPreview ? "Keep sidebar open" : "Collapse sidebar"
                }
                className="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded border border-transparent text-fg-2 transition-colors hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg focus:border-sidebar-selected-border focus:bg-sidebar-selected/40 focus:text-fg focus:outline-none"
              >
                <PanelToggleGlyph
                  side="left"
                  filled={!collapsed}
                  className="h-[12px] w-[15.4px]"
                />
              </button>
            </div>
          </div>
        ) : null}

        {sidebarVisible ? (
          <div
            onPointerDown={handleResizeStart}
            title="Drag to resize"
            className="absolute right-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
          />
        ) : null}
      </aside>

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
        onClose={() => {
          setCreatingChat(false);
          setNewChatFolderId(null);
        }}
        onStarted={(spawned) => {
          setCreatingChat(false);
          const targetFolderId = newChatFolderId;
          setNewChatFolderId(null);
          if (targetFolderId) {
            void moveSessionTabToFolder(spawned.id, targetFolderId)
              .catch((error) =>
                console.error(
                  "sidebar: create chat tab in folder failed",
                  error,
                ),
              )
              .finally(() => {
                navigate(`/chats/${spawned.id}`, {
                  state: { sessionStatus: "running" },
                });
              });
            return;
          }
          const chatLayout = activatePaneLayoutForSession(currentChatSessionId);
          const targetPaneId = newChatTargetPane(
            chatLayout,
            currentChatSessionId,
          );
          if (targetPaneId) {
            const memberIds = visibleSessionIds(chatLayout.root);
            assignSessionToPane(targetPaneId, spawned.id);
            focusPane(targetPaneId);
            reportSubjectsNow(
              visibleSessionIds(getPaneLayout(spawned.id).root).map(
                (value) => ({
                  type: "DirectChat",
                  value,
                }),
              ),
            );
            if (
              shouldInheritPinOnAdd(
                memberIds,
                pinnedSessionIds(directSessions),
                spawned.id,
              )
            ) {
              void api.session
                .pin(spawned.id, true)
                .then(() => refreshDirectSessions())
                .catch((e) =>
                  console.error("sidebar: session_pin on group add failed", e),
                );
            }
          }
          navigate(`/chats/${spawned.id}`, {
            state: { sessionStatus: "running" },
          });
        }}
      />

      {chatTabMenu ? (
        <RowContextMenu
          pinned={chatTabMenu.members.every((member) => member.pinned)}
          anchorX={chatTabMenu.x}
          anchorY={chatTabMenu.y}
          renameLabel="Rename tab"
          archiveLabel="Archive all"
          folders={folders}
          currentFolderId={chatTabMenu.layout.folderId}
          onMoveToFolder={(folderId) => {
            void moveTabToFolder(chatTabMenu.layout.id, folderId).catch((e) =>
              console.error("sidebar: tab_move_to_folder failed", e),
            );
            closeChatTabMenu();
          }}
          onClose={closeChatTabMenu}
          onPin={() => {
            void setChatTabPin(
              chatTabMenu.members,
              !chatTabMenu.members.every((member) => member.pinned),
            );
            closeChatTabMenu();
          }}
          onRename={() => {
            renameChatTab(chatTabMenu.members);
            closeChatTabMenu();
          }}
          onOpenInNewWindow={() => {
            openChatTabInNewWindow(chatTabMenu.members);
            closeChatTabMenu();
          }}
          onArchive={() => {
            void archiveChatTab(chatTabMenu.members);
            closeChatTabMenu();
          }}
        />
      ) : null}

      {folderMenu ? (
        <FolderContextMenu
          anchorX={folderMenu.x}
          anchorY={folderMenu.y}
          onClose={closeFolderMenu}
          onRename={() => {
            void renameFolder(folderMenu.id, folderMenu.name);
            closeFolderMenu();
          }}
          onDelete={() => {
            requestFolderDelete(folderMenu.id, folderMenu.name);
            closeFolderMenu();
          }}
        />
      ) : null}

      {chatCreateMenu ? (
        <ChatCreateContextMenu
          anchorX={chatCreateMenu.x}
          anchorY={chatCreateMenu.y}
          onClose={() => setChatCreateMenu(null)}
          onNewChat={() => {
            setChatCreateMenu(null);
            handleNewDirectChat();
          }}
          onNewFolder={beginFolderCreate}
        />
      ) : null}

      <ConfirmDialog
        open={folderDeleteConfirm !== null}
        title={`Delete folder "${folderDeleteConfirm?.name ?? ""}"?`}
        body={`This folder contains ${folderDeleteConfirm?.count ?? 0} tab${folderDeleteConfirm?.count === 1 ? "" : "s"}. Deleting it archives every chat in those tabs and removes the folder. The chats will appear in Settings → Archived.`}
        confirmLabel="Delete folder"
        busyLabel="Archiving…"
        busy={deletingFolder}
        onConfirm={() => {
          if (!folderDeleteConfirm) return;
          void deleteFolder(folderDeleteConfirm);
        }}
        onCancel={() => setFolderDeleteConfirm(null)}
      />

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
            : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
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

// ---- collapsible section header ---------------------------------------

function CollapsibleSectionHeader({
  label,
  open,
  onToggle,
  onPlus,
  plusTitle,
  plusExpanded,
  plusPopup,
  plusMenu,
  onPlusMenuClose,
  attention,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
  onPlus: () => void;
  plusTitle: string;
  /** Pass the popup's open state so the trigger advertises its expanded
   *  state and the correct dialog/menu relationship. */
  plusExpanded?: boolean;
  plusPopup?: "dialog" | "menu";
  plusMenu?: ReactNode;
  onPlusMenuClose?: () => void;
  attention?: ChatAttentionState;
}) {
  const plusRef = useRef<HTMLButtonElement>(null);
  return (
    <div className="flex items-center justify-between gap-2 px-5 pb-1.5">
      <button
        type="button"
        onClick={onToggle}
        className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3 hover:text-fg-2"
      >
        <span>{label}</span>
        <ChevronDown
          aria-hidden
          className={`h-2.5 w-2.5 transition-transform ${
            open ? "" : "-rotate-90"
          }`}
        />
      </button>
      <div className="flex items-center gap-1.5">
        {attention !== undefined ? (
          <ChatAttentionIndicator state={attention} />
        ) : null}
        <button
          ref={plusRef}
          type="button"
          onClick={onPlus}
          title={plusTitle}
          aria-label={plusTitle}
          aria-haspopup={
            plusExpanded === undefined ? undefined : (plusPopup ?? "dialog")
          }
          aria-expanded={plusExpanded}
          className="cursor-pointer rounded p-1 text-fg-2 transition-colors hover:bg-bg hover:text-fg"
        >
          <Plus aria-hidden className="h-3 w-3" />
        </button>
      </div>
      {plusMenu && onPlusMenuClose ? (
        <PopoverMenu
          open={plusExpanded === true}
          anchorRef={plusRef}
          onClose={onPlusMenuClose}
          minWidth={160}
        >
          {plusMenu}
        </PopoverMenu>
      ) : null}
    </div>
  );
}

// ---- sidebar list rows ------------------------------------------------

function TabDropDivider({
  active,
  onDragOver,
  onDrop,
}: {
  active: boolean;
  onDragOver: (event: DragEvent<HTMLDivElement>) => void;
  onDrop: (event: DragEvent<HTMLDivElement>) => void;
}) {
  return (
    <div
      onDragOver={onDragOver}
      onDrop={onDrop}
      className="relative z-20 -my-1 h-2 shrink-0"
    >
      {active ? (
        <>
          <span className="absolute left-0 top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent" />
          <span className="absolute inset-x-0 top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-accent" />
        </>
      ) : null}
    </div>
  );
}

function NewFolderRow({
  onSubmit,
  onCancel,
}: {
  onSubmit: (name: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const submittingRef = useRef(false);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const submit = useCallback(async () => {
    const name = draft.trim();
    if (!name) {
      onCancel();
      return;
    }
    if (submittingRef.current) return;
    submittingRef.current = true;
    try {
      await onSubmit(name);
    } catch {
      submittingRef.current = false;
      inputRef.current?.focus();
    }
  }, [draft, onCancel, onSubmit]);

  return (
    <div
      onContextMenu={(event) => event.stopPropagation()}
      className="flex items-center gap-1.5 rounded border border-sidebar-selected-border bg-sidebar-selected px-2.5 py-1.5 text-xs shadow-sm"
    >
      <ChevronDown aria-hidden className="h-3 w-3 shrink-0 text-fg-2" />
      <Folder aria-hidden className="h-3 w-3 shrink-0 text-fg-2" />
      <input
        ref={inputRef}
        value={draft}
        placeholder="Folder name"
        aria-label="Folder name"
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void submit();
          } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => void submit()}
        className="min-w-0 flex-1 bg-transparent text-xs text-fg outline-none placeholder:text-fg-3"
      />
    </div>
  );
}

function SidebarListRow({
  selected,
  accentBar,
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
  /** 2px accent bar on the row's left edge — marks the chat in the
   *  focused split pane (impl 0020), mirroring the pane focus ring. */
  accentBar?: boolean;
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
      className={`group relative flex w-full items-center gap-2 rounded border px-2.5 py-1.5 text-left text-xs transition-colors ${
        selected
          ? "border-sidebar-selected-border bg-sidebar-selected font-semibold text-fg shadow-sm"
          : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
      }`}
    >
      {accentBar ? (
        <span
          aria-hidden
          className="absolute inset-y-0.5 left-0 w-0.5 rounded-full bg-accent"
        />
      ) : null}
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
  const activity = mission.any_session_live ? (mission.activity ?? "busy") : null;
  const statusLabel = activity ?? "paused";
  const tooltip = `${mission.crew_name || "Mission"} · ${statusLabel}${
    mission.pinned_at ? " · pinned" : ""
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

export function SessionRow({
  session,
  activity,
  selected,
  paneFocused,
  renaming,
  onClick,
  onContextMenu,
  onRenameSubmit,
  onRenameCancel,
}: {
  session: DirectSessionEntry;
  activity: SessionActivityState | undefined;
  selected: boolean;
  /** This chat sits in the focused split pane (impl 0020). */
  paneFocused?: boolean;
  renaming: boolean;
  onClick: () => void;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  onRenameSubmit: (nextTitle: string | null) => void;
  onRenameCancel: () => void;
}) {
  const defaultLabel = session.handle
    ? `@${session.handle} · ${formatStartedAt(session)}`
    : `${session.display_name} · ${formatStartedAt(session)}`;
  const label = session.title ?? defaultLabel;
  const dim = session.status !== "running";
  const displayStatus = directChatDisplayStatus(session, activity);
  const tooltip = `${session.handle ? `@${session.handle}` : session.display_name} · ${displayStatus}${
    session.status !== "running" && session.resumable ? " · resumable" : ""
  }${session.pinned ? " · pinned" : ""}`;

  return (
    <SidebarListRow
      selected={selected}
      accentBar={paneFocused}
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
  renameLabel = "Rename",
  archiveLabel = "Archive",
  folders,
  currentFolderId,
  onMoveToFolder,
}: {
  pinned: boolean;
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onOpenInNewWindow: () => void;
  onArchive: () => void;
  renameLabel?: string;
  archiveLabel?: string;
  folders?: { id: string; name: string }[];
  currentFolderId?: string | null;
  onMoveToFolder?: (folderId: string | null) => void;
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
      <ContextMenuItem icon={SquarePen} label={renameLabel} onClick={onRename} />
      <ContextMenuItem
        icon={AppWindow}
        label="Open in New Window"
        onClick={onOpenInNewWindow}
      />
      {folders && onMoveToFolder ? (
        <>
          <div className="my-1 h-px bg-line" />
          {currentFolderId !== null ? (
            <ContextMenuItem
              icon={Folder}
              label="Move out of folder"
              onClick={() => onMoveToFolder(null)}
            />
          ) : null}
          {folders
            .filter((folder) => folder.id !== currentFolderId)
            .map((folder) => (
              <ContextMenuItem
                key={folder.id}
                icon={Folder}
                label={`Move to ${folder.name}`}
                onClick={() => onMoveToFolder(folder.id)}
              />
            ))}
        </>
      ) : null}
      <ContextMenuItem
        icon={Archive}
        label={archiveLabel}
        onClick={onArchive}
        danger
      />
    </div>
  );
}

function FolderContextMenu({
  anchorX,
  anchorY,
  onClose,
  onRename,
  onDelete,
}: {
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: anchorX, y: anchorY });
  useEffect(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(anchorX, window.innerWidth - rect.width - 4)),
      y: Math.max(4, Math.min(anchorY, window.innerHeight - rect.height - 4)),
    });
  }, [anchorX, anchorY]);
  useEffect(() => {
    const onMouseDown = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      style={{ position: "fixed", left: pos.x, top: pos.y, width: 180 }}
      className="z-50 flex flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
    >
      <ContextMenuItem icon={SquarePen} label="Rename folder" onClick={onRename} />
      <ContextMenuItem icon={Trash2} label="Delete" onClick={onDelete} danger />
    </div>
  );
}

function ChatCreateContextMenu({
  anchorX,
  anchorY,
  onClose,
  onNewChat,
  onNewFolder,
}: {
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onNewChat: () => void;
  onNewFolder: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x: anchorX, y: anchorY });
  useEffect(() => {
    if (!ref.current) return;
    const rect = ref.current.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(anchorX, window.innerWidth - rect.width - 4)),
      y: Math.max(4, Math.min(anchorY, window.innerHeight - rect.height - 4)),
    });
  }, [anchorX, anchorY]);
  useEffect(() => {
    const onMouseDown = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
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
        icon={MessageSquarePlus}
        label="New chat"
        onClick={onNewChat}
      />
      <ContextMenuItem
        icon={FolderPlus}
        label="New folder"
        onClick={onNewFolder}
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
