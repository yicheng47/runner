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
  type ReactNode,
} from "react";
import {
  DndContext,
  DragOverlay,
  PointerSensor,
  pointerWithin,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragOverEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import {
  NavLink,
  useLocation,
  useMatch,
  useNavigate,
} from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { basename } from "@tauri-apps/api/path";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AppWindow,
  Archive,
  ChevronDown,
  ChevronRight,
  Flag,
  Folder,
  FolderCode,
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

import {
  api,
  type DirectSessionEntry,
  type ProjectRow,
} from "../lib/api";
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
  chatTabArchiveLabel,
  chatTabIsLive,
  isChatTabDropIndexAllowed,
  orderedChatTabIdsAfterDrop,
} from "../lib/chatTabs";
import {
  missionAttentionState,
  rollupAttentionState,
  tabAttentionState,
  useDirectSessionActivity,
  type ChatAttentionState,
} from "../lib/chatAttention";
import {
  activatePaneLayoutForSession,
  assignSessionToPane,
  createChatFolder,
  focusPane,
  getPaneLayout,
  leafForSession,
  newChatTargetPane,
  removeArchivedSessionFromLayout,
  hydratePaneLayoutsFromDb,
  moveSessionTabToFolder,
  reorderTab,
  setGroupNameForSession,
  useFolders,
  usePaneLayouts,
  visibleSessionIds,
  type PaneLayout,
} from "../lib/paneLayout";
import { ChatTabGroup } from "./ChatTabGroup";
import {
  ChatAttentionIndicator,
  SidebarTabIcon,
  SidebarTabRow,
} from "./SidebarTabRow";
import { PanelToggleGlyph } from "./PanelToggleGlyph";
import { PopoverMenu } from "./ui/PopoverMenu";
import { useResizableWidth } from "../hooks/useResizableWidth";
import {
  BRAND_MARK_PINNED_COLOR,
  readBrandTint,
  STORAGE_APP_BRAND_TINT,
} from "../lib/settings";
import { reportSubjectsNow } from "../lib/windowFocus";
import {
  projectIdForTab,
  setActiveProjectScope,
} from "../lib/projectScope";
import type {
  AppendedEvent,
  MissionSummary,
  SessionActivityState,
} from "../lib/types";
import { StartMissionModal } from "./StartMissionModal";
import { StartChatModal } from "./StartChatModal";
import { CommandPalette } from "./CommandPalette";
import { UpdatePromptCard } from "./UpdatePromptCard";
import { ConfirmDialog } from "./settings/ConfirmDialog";
import { eventMatchesShortcut } from "../lib/keymap";

const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 480;
const SIDEBAR_DEFAULT = 240;
const STORAGE_WIDTH = "runner.sidebar.width";
const STORAGE_PROJECTS_OPEN = "runner.sidebar.projects.open";
const STORAGE_MISSION_OPEN = "runner.sidebar.mission.open";
const STORAGE_SESSION_OPEN = "runner.sidebar.session.open";
const SIDEBAR_NAVIGATE_EVENT = "runner:navigate-sidebar-page";
const SIDEBAR_NAVIGATION_HISTORY_LIMIT = 64;

type TabDropTarget = {
  folderId: string | null;
  index: number;
  markerKey: string;
};

type ChatTabDndData =
  | {
      kind: "tab";
      tabId: string;
      folderId: string | null;
    }
  | ({ kind: "position" } & TabDropTarget)
  | {
      kind: "collapsed-folder";
      folderId: string;
    };

const tabDndId = (tabId: string) => `chat-tab:${tabId}`;
const tabDropDndId = (folderId: string | null, markerKey: string) =>
  `chat-tab-drop:${folderId ?? "ungrouped"}:${markerKey}`;
const collapsedFolderDndId = (folderId: string) =>
  `chat-folder-drop:${folderId}`;

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
  return eventMatchesShortcut(e, "page-previous")
    ? "previous"
    : eventMatchesShortcut(e, "page-next")
      ? "next"
      : null;
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
  // Settings takeover (see AppShell): the sidebar stays mounted but
  // hidden while `/settings` is shown, so its global shortcuts must go
  // inert — Cmd+T here would spawn a chat under the takeover.
  const settingsActive = location.pathname.startsWith("/settings");
  const tabDragSensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 5 },
    }),
  );

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
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [projectsLoaded, setProjectsLoaded] = useState(false);
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
  const [projectsOpen, setProjectsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_PROJECTS_OPEN, true),
  );
  const [missionsOpen, setMissionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_MISSION_OPEN, true),
  );
  const [sessionsOpen, setSessionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_SESSION_OPEN, true),
  );

  const [creatingMission, setCreatingMission] = useState(false);
  const [newMissionProjectId, setNewMissionProjectId] = useState<
    string | null
  >(null);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [renamingProjectId, setRenamingProjectId] = useState<string | null>(
    null,
  );
  const [projectMenu, setProjectMenu] = useState<{
    project: ProjectRow;
    x: number;
    y: number;
  } | null>(null);
  const [projectDeleteConfirm, setProjectDeleteConfirm] =
    useState<ProjectRow | null>(null);
  const [deletingProject, setDeletingProject] = useState(false);

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
  const [collapsedFolderIds, setCollapsedFolderIds] = useState<Set<string>>(
    () => new Set(),
  );
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
  // CHAT creation state. The `+` and empty-space context menus can start a
  // chat or insert a focused inline folder-name row.
  const [creatingChat, setCreatingChat] = useState(false);
  const [newChatProjectId, setNewChatProjectId] = useState<string | null>(null);
  const [newChatFolderId, setNewChatFolderId] = useState<string | null>(null);
  const [chatAddMenuOpen, setChatAddMenuOpen] = useState(false);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [renamingChatTabId, setRenamingChatTabId] = useState<string | null>(
    null,
  );
  const [renamingFolderId, setRenamingFolderId] = useState<string | null>(null);
  const [chatCreateMenu, setChatCreateMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const [collapsedFolderDropId, setCollapsedFolderDropId] = useState<
    string | null
  >(null);
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [tabDropTarget, setTabDropTarget] = useState<TabDropTarget | null>(
    null,
  );
  const tabDropTargetRef = useRef<TabDropTarget | null>(null);

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
          projectId: projectIdForTab(members),
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
  const projectTabItems = useMemo(
    () => tabItems.filter((item) => item.projectId !== null),
    [tabItems],
  );
  const chatSectionTabItems = useMemo(
    () => tabItems.filter((item) => item.projectId === null),
    [tabItems],
  );
  const projectMissions = useMemo(
    () => missions.filter((mission) => mission.project_id !== null),
    [missions],
  );
  const ungroupedMissions = useMemo(
    () => missions.filter((mission) => mission.project_id === null),
    [missions],
  );
  const chatAttention = useMemo(
    () =>
      rollupAttentionState(
        chatSectionTabItems.map((item) => item.attention),
      ),
    [chatSectionTabItems],
  );
  const missionAttention = useMemo(
    () =>
      rollupAttentionState(
        ungroupedMissions.map((mission) =>
          missionAttentionState(
            mission.any_session_live,
            mission.activity,
          ),
        ),
      ),
    [ungroupedMissions],
  );
  const projectAttention = useMemo(
    () =>
      rollupAttentionState([
        ...projectTabItems.map((item) => item.attention),
        ...projectMissions.map((mission) =>
          missionAttentionState(
            mission.any_session_live,
            mission.activity,
          ),
        ),
      ]),
    [projectMissions, projectTabItems],
  );
  const draggedTabItem = chatSectionTabItems.find(
    (item) => item.layout.id === draggedTabId,
  );
  const orderedChatRows = useMemo(
    () => tabItems.map((item) => item.members[0]),
    [tabItems],
  );
  const activeProject = useMemo(
    () => projects.find((project) => project.id === activeProjectId) ?? null,
    [activeProjectId, projects],
  );
  const newChatProject = useMemo(
    () => projects.find((project) => project.id === newChatProjectId) ?? null,
    [newChatProjectId, projects],
  );
  const newMissionProject = useMemo(
    () =>
      projects.find((project) => project.id === newMissionProjectId) ?? null,
    [newMissionProjectId, projects],
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

  const refreshProjects = useCallback(async () => {
    try {
      const rows = await api.project.list();
      setProjects(rows);
      setProjectsLoaded(true);
    } catch (e) {
      console.error("sidebar: refreshProjects failed", e);
    }
  }, []);

  useEffect(() => {
    void refreshProjects();
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen("project/changed", () => {
      void refreshProjects();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshProjects]);

  useEffect(() => {
    if (
      projectsLoaded &&
      activeProjectId &&
      !projects.some((project) => project.id === activeProjectId)
    ) {
      setActiveProjectId(null);
    }
  }, [activeProjectId, projects, projectsLoaded]);

  useEffect(() => {
    setActiveProjectScope(activeProject);
  }, [activeProject]);

  useEffect(() => {
    if (!projectsLoaded) return;
    if (currentMissionId) {
      const mission = missions.find((item) => item.id === currentMissionId);
      if (mission) {
        setActiveProjectId(
          mission.project_id &&
            projects.some((project) => project.id === mission.project_id)
            ? mission.project_id
            : null,
        );
      }
      return;
    }
    if (currentChatSessionId) {
      const session = directSessions.find(
        (item) => item.session_id === currentChatSessionId,
      );
      if (session) {
        setActiveProjectId(
          session.project_id &&
            projects.some((project) => project.id === session.project_id)
            ? session.project_id
            : null,
        );
      }
    }
  }, [
    currentChatSessionId,
    currentMissionId,
    directSessions,
    missions,
    projects,
    projectsLoaded,
  ]);

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

  // Skip while editing text controls so shortcuts don't hijack form
  // input. xterm's hidden textarea is not an editor field from the
  // app's point of view, so Meta shortcuts still win there; Ctrl
  // shortcuts stay with the PTY/TUI.
  useEffect(() => {
    if (settingsActive) return;
    const onKey = (e: KeyboardEvent) => {
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
      if (eventMatchesShortcut(e, "command-palette")) {
        e.preventDefault();
        e.stopPropagation();
        setPaletteOpen(true);
      } else if (eventMatchesShortcut(e, "new-chat")) {
        e.preventDefault();
        e.stopPropagation();
        setNewChatProjectId(activeProjectId);
        setCreatingChat(true);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [activeProjectId, settingsActive]);

  useEffect(() => {
    if (settingsActive) return;
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
  }, [navigateSidebarPage, settingsActive]);

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
      setProjectMenu(null);
      setChatTabMenu({ layout, members, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeChatTabMenu = useCallback(() => setChatTabMenu(null), []);

  const openMissionMenu = useCallback(
    (mission: MissionSummary, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setFolderMenu(null);
      setProjectMenu(null);
      setMissionMenu({ mission, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeFolderMenu = useCallback(() => setFolderMenu(null), []);
  const openFolderMenu = useCallback(
    (
      folder: { id: string; name: string },
      anchor: { x: number; y: number },
    ) => {
      setChatTabMenu(null);
      setMissionMenu(null);
      setProjectMenu(null);
      setFolderMenu({
        id: folder.id,
        name: folder.name,
        x: anchor.x,
        y: anchor.y,
      });
    },
    [],
  );
  const closeMissionMenu = useCallback(() => setMissionMenu(null), []);
  const openProjectMenu = useCallback(
    (project: ProjectRow, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setMissionMenu(null);
      setFolderMenu(null);
      setProjectMenu({ project, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeProjectMenu = useCallback(() => setProjectMenu(null), []);

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

  const submitProjectRename = useCallback(
    async (id: string, currentName: string, nextName: string) => {
      const trimmed = nextName.trim();
      setRenamingProjectId(null);
      if (!trimmed || trimmed === currentName.trim()) return;
      try {
        await api.project.rename(id, trimmed);
        await refreshProjects();
      } catch (e) {
        console.error("sidebar: project_rename failed", e);
      }
    },
    [refreshProjects],
  );

  const toggleProject = useCallback((project: ProjectRow) => {
    setActiveProjectId(project.id);
    setCollapsedProjectIds((current) => {
      const next = new Set(current);
      if (next.has(project.id)) next.delete(project.id);
      else next.add(project.id);
      return next;
    });
  }, []);

  const deleteProject = useCallback(
    async (project: ProjectRow) => {
      setDeletingProject(true);
      try {
        await api.project.delete(project.id);
        setProjectDeleteConfirm(null);
        if (activeProjectId === project.id) setActiveProjectId(null);
        await Promise.all([
          refreshProjects(),
          refreshMissions(),
          refreshDirectSessions(),
        ]);
      } catch (e) {
        console.error("sidebar: project_delete failed", e);
      } finally {
        setDeletingProject(false);
      }
    },
    [
      activeProjectId,
      refreshDirectSessions,
      refreshMissions,
      refreshProjects,
    ],
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

  const submitChatTabRename = useCallback(
    (sessionId: string, nextName: string) => {
      setRenamingChatTabId(null);
      setGroupNameForSession(sessionId, nextName);
    },
    [],
  );

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
      setProjectMenu(null);
      setChatCreateMenu(anchor);
    },
    [],
  );

  const submitFolderRename = useCallback(
    async (id: string, currentName: string, nextName: string) => {
      const trimmed = nextName.trim();
      setRenamingFolderId(null);
      if (!trimmed || trimmed === currentName.trim()) return;
      try {
        await api.folder.rename(id, trimmed);
        await hydratePaneLayoutsFromDb();
      } catch (e) {
        console.error("sidebar: folder_rename failed", e);
      }
    },
    [],
  );

  const toggleFolder = useCallback((id: string) => {
    setCollapsedFolderIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
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
    setNewChatProjectId(null);
    setCreatingChat(true);
  }, []);

  const handleNewFolderChat = useCallback((folderId: string) => {
    setChatAddMenuOpen(false);
    setChatCreateMenu(null);
    setCreatingFolder(false);
    setNewChatFolderId(folderId);
    setNewChatProjectId(null);
    setCreatingChat(true);
  }, []);

  const handleNewProjectChat = useCallback((projectId: string) => {
    setChatCreateMenu(null);
    setCreatingFolder(false);
    setNewChatFolderId(null);
    setNewChatProjectId(projectId);
    setActiveProjectId(projectId);
    setCreatingChat(true);
  }, []);

  const handleNewProjectMission = useCallback((projectId: string) => {
    setNewMissionProjectId(projectId);
    setActiveProjectId(projectId);
    setCreatingMission(true);
  }, []);

  const addProject = useCallback(async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Add a project",
      });
      if (typeof picked !== "string") return;
      const project = await api.project.create(await basename(picked), picked);
      setProjectsOpen(true);
      setStoredFlag(STORAGE_PROJECTS_OPEN, true);
      setActiveProjectId(project.id);
      await refreshProjects();
    } catch (e) {
      console.error("sidebar: project_create failed", e);
    }
  }, [refreshProjects]);

  const clearTabDrag = useCallback(() => {
    setDraggedTabId(null);
    setCollapsedFolderDropId(null);
    setTabDropTarget(null);
    tabDropTargetRef.current = null;
  }, []);

  const commitTabDrop = useCallback(
    async (tabId: string, folderId: string | null, requestedIndex: number) => {
      const dragged = chatSectionTabItems.find(
        (item) => item.layout.id === tabId,
      );
      if (!dragged) {
        clearTabDrag();
        return;
      }
      const pinned = dragged.members.every((member) => member.pinned);
      const targetItems = chatSectionTabItems.filter(
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
    [chatSectionTabItems, clearTabDrag],
  );

  const resolveTabDropTarget = useCallback(
    (event: DragOverEvent | DragEndEvent): TabDropTarget | null => {
      const activeData = event.active.data.current as
        | ChatTabDndData
        | undefined;
      const overData = event.over?.data.current as
        | ChatTabDndData
        | undefined;
      if (activeData?.kind !== "tab" || !overData) return null;

      const dragged = chatSectionTabItems.find(
        (item) => item.layout.id === activeData.tabId,
      );
      if (!dragged) return null;

      if (overData.kind === "position") {
        const targetItems = chatSectionTabItems.filter(
          (item) => item.layout.folderId === overData.folderId,
        );
        const allowed = isChatTabDropIndexAllowed(
          targetItems.map((item) => ({
            id: item.layout.id,
            pinned: item.members.every((member) => member.pinned),
          })),
          dragged.layout.id,
          dragged.members.every((member) => member.pinned),
          overData.index,
        );
        return allowed
          ? {
              folderId: overData.folderId,
              index: overData.index,
              markerKey: overData.markerKey,
            }
          : null;
      }

      if (overData.kind !== "tab") return null;
      const targetItems = chatSectionTabItems.filter(
        (item) => item.layout.folderId === overData.folderId,
      );
      const overIndex = targetItems.findIndex(
        (item) => item.layout.id === overData.tabId,
      );
      if (overIndex < 0 || !event.over) return null;

      const activeRect =
        event.active.rect.current.translated ??
        event.active.rect.current.initial;
      const after = activeRect
        ? activeRect.top + activeRect.height / 2 >=
          event.over.rect.top + event.over.rect.height / 2
        : false;
      const originalIndex = overIndex + (after ? 1 : 0);
      const index = targetItems
        .slice(0, originalIndex)
        .filter((item) => item.layout.id !== dragged.layout.id).length;
      if (
        !isChatTabDropIndexAllowed(
          targetItems.map((item) => ({
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

      return {
        folderId: overData.folderId,
        index,
        markerKey:
          originalIndex < targetItems.length
            ? `before-${targetItems[originalIndex].layout.id}`
            : `after-${overData.folderId ?? "ungrouped"}`,
      };
    },
    [chatSectionTabItems],
  );

  const resolveCollapsedFolderDropTarget = useCallback(
    (tabId: string, folderId: string): TabDropTarget | null => {
      const dragged = chatSectionTabItems.find(
        (item) => item.layout.id === tabId,
      );
      if (!dragged) return null;
      const targetItems = chatSectionTabItems.filter(
        (item) => item.layout.folderId === folderId,
      );
      const orderedIds = orderedChatTabIdsAfterDrop(
        targetItems.map((item) => ({
          id: item.layout.id,
          pinned: item.members.every((member) => member.pinned),
        })),
        tabId,
        dragged.members.every((member) => member.pinned),
        Number.MAX_SAFE_INTEGER,
      );
      const index = orderedIds.indexOf(tabId);
      const remaining = targetItems.filter(
        (item) => item.layout.id !== tabId,
      );
      if (index < 0) return null;
      return {
        folderId,
        index,
        markerKey:
          index < remaining.length
            ? `before-${remaining[index].layout.id}`
            : `after-${folderId}`,
      };
    },
    [chatSectionTabItems],
  );

  const handleTabDragStart = useCallback((event: DragStartEvent) => {
    const data = event.active.data.current as ChatTabDndData | undefined;
    if (data?.kind !== "tab") return;
    setDraggedTabId(data.tabId);
    setCollapsedFolderDropId(null);
    setTabDropTarget(null);
    tabDropTargetRef.current = null;
  }, []);

  const handleTabDragOver = useCallback(
    (event: DragOverEvent) => {
      const activeData = event.active.data.current as
        | ChatTabDndData
        | undefined;
      const overData = event.over?.data.current as
        | ChatTabDndData
        | undefined;
      if (
        activeData?.kind === "tab" &&
        overData?.kind === "collapsed-folder"
      ) {
        const target = resolveCollapsedFolderDropTarget(
          activeData.tabId,
          overData.folderId,
        );
        setCollapsedFolderDropId(overData.folderId);
        setTabDropTarget(target);
        tabDropTargetRef.current = target;
        return;
      }

      setCollapsedFolderDropId((folderId) => {
        if (
          folderId &&
          (overData?.kind === "position" || overData?.kind === "tab") &&
          overData.folderId === folderId
        ) {
          return folderId;
        }
        return null;
      });
      const target = resolveTabDropTarget(event);
      setTabDropTarget(target);
      tabDropTargetRef.current = target;
    },
    [resolveCollapsedFolderDropTarget, resolveTabDropTarget],
  );

  const handleTabDragEnd = useCallback(
    (event: DragEndEvent) => {
      const activeData = event.active.data.current as
        | ChatTabDndData
        | undefined;
      const overData = event.over?.data.current as
        | ChatTabDndData
        | undefined;
      if (activeData?.kind !== "tab") {
        clearTabDrag();
        return;
      }
      if (overData?.kind === "collapsed-folder") {
        void commitTabDrop(
          activeData.tabId,
          overData.folderId,
          Number.MAX_SAFE_INTEGER,
        );
        return;
      }
      const target = resolveTabDropTarget(event) ?? tabDropTargetRef.current;
      if (target) {
        void commitTabDrop(activeData.tabId, target.folderId, target.index);
      } else {
        clearTabDrag();
      }
    },
    [clearTabDrag, commitTabDrop, resolveTabDropTarget],
  );

  const toggleMissions = useCallback(() => {
    setMissionsOpen((prev) => {
      setStoredFlag(STORAGE_MISSION_OPEN, !prev);
      return !prev;
    });
  }, []);

  const toggleProjects = useCallback(() => {
    setProjectsOpen((prev) => {
      setStoredFlag(STORAGE_PROJECTS_OPEN, !prev);
      return !prev;
    });
  }, []);

  const toggleSessions = useCallback(() => {
    setSessionsOpen((prev) => {
      setStoredFlag(STORAGE_SESSION_OPEN, !prev);
      return !prev;
    });
  }, []);

  const renderTabItem = (
    item: (typeof tabItems)[number],
    draggable = true,
  ) => {
    const active = item.members.some(
      (member) => member.session_id === currentChatSessionId,
    );
    const row = (
      <ChatTabGroup
        layout={item.layout}
        members={item.members}
        active={active}
        attention={item.attention}
        onActivate={(entry) => {
          setActiveProjectId(item.projectId);
          activateTabChat(item.layout.id, item.members, entry);
        }}
        onContextMenu={(anchor) =>
          openChatTabMenu(item.layout, item.members, anchor)
        }
        dragging={draggedTabId === item.layout.id}
        renaming={renamingChatTabId === item.layout.id}
        onRenameSubmit={(nextName) =>
          submitChatTabRename(item.members[0].session_id, nextName)
        }
        onRenameCancel={() => setRenamingChatTabId(null)}
      />
    );
    if (!draggable) return <Fragment key={item.layout.id}>{row}</Fragment>;
    return (
      <SortableChatTab
        key={item.layout.id}
        tabId={item.layout.id}
        folderId={item.layout.folderId}
        disabled={renamingChatTabId === item.layout.id}
      >
        {row}
      </SortableChatTab>
    );
  };

  const renderTabDropDivider = (
    folderId: string | null,
    items: typeof tabItems,
    originalIndex: number,
    key: string,
  ) => {
    const dragged = chatSectionTabItems.find(
      (item) => item.layout.id === draggedTabId,
    );
    const index = items
      .slice(0, originalIndex)
      .filter((item) => item.layout.id !== draggedTabId).length;
    const enabled = Boolean(
      dragged &&
        isChatTabDropIndexAllowed(
          items.map((item) => ({
            id: item.layout.id,
            pinned: item.members.every((member) => member.pinned),
          })),
          dragged.layout.id,
          dragged.members.every((member) => member.pinned),
          index,
        ),
    );
    return (
      <TabDropDivider
        key={key}
        id={tabDropDndId(folderId, key)}
        enabled={enabled}
        folderId={folderId}
        index={index}
        markerKey={key}
        active={
          tabDropTarget?.folderId === folderId &&
          tabDropTarget.index === index &&
          tabDropTarget.markerKey === key
        }
      />
    );
  };

  const renderTabItems = (
    items: typeof tabItems,
    folderId: string | null,
    draggable = true,
  ) => {
    if (!draggable) return items.map((item) => renderTabItem(item, false));
    if (items.length === 0) {
      const markerKey = `after-${folderId ?? "ungrouped"}`;
      return (
        <SortableContext items={[]} strategy={verticalListSortingStrategy}>
          <EmptyTabDropArea
            id={tabDropDndId(folderId, markerKey)}
            enabled={draggedTabId !== null}
            folderId={folderId}
            markerKey={markerKey}
            active={
              tabDropTarget?.folderId === folderId &&
              tabDropTarget.index === 0 &&
              tabDropTarget.markerKey === markerKey
            }
            label={folderId === null ? null : "Empty folder"}
          />
        </SortableContext>
      );
    }

    return (
      <SortableContext
        items={items.map((item) => tabDndId(item.layout.id))}
        strategy={verticalListSortingStrategy}
      >
        {items.map((item, originalIndex) => (
          <Fragment key={item.layout.id}>
            {renderTabDropDivider(
              folderId,
              items,
              originalIndex,
              `before-${item.layout.id}`,
            )}
            {renderTabItem(item)}
          </Fragment>
        ))}
        {renderTabDropDivider(
          folderId,
          items,
          items.length,
          `after-${folderId ?? "ungrouped"}`,
        )}
      </SortableContext>
    );
  };

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

            {/* Projects and Mission are capped independent trays; Chat fills
                the remaining height. */}
            <div className="flex min-h-0 flex-1 flex-col pb-3">
              <section className="flex shrink-0 flex-col">
                <CollapsibleSectionHeader
                  label="PROJECT"
                  open={projectsOpen}
                  attention={projectsOpen ? null : projectAttention}
                  onToggle={toggleProjects}
                  onPlus={() => void addProject()}
                  plusTitle="Add project"
                />
                {projectsOpen ? (
                  <div className="flex max-h-[34vh] flex-col gap-0.5 overflow-y-auto px-3 pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
                    {projects.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No projects yet.
                      </p>
                    ) : (
                      projects.map((project) => {
                        const nestedMissions = projectMissions.filter(
                          (mission) => mission.project_id === project.id,
                        );
                        const nestedTabs = projectTabItems.filter(
                          (item) => item.projectId === project.id,
                        );
                        const nestedAttention = rollupAttentionState([
                          ...nestedTabs.map((item) => item.attention),
                          ...nestedMissions.map((mission) =>
                            missionAttentionState(
                              mission.any_session_live,
                              mission.activity,
                            ),
                          ),
                        ]);
                        const live =
                          nestedMissions.some(
                            (mission) => mission.any_session_live,
                          ) ||
                          nestedTabs.some((item) =>
                            chatTabIsLive(item.members),
                          );
                        const projectCollapsed = collapsedProjectIds.has(
                          project.id,
                        );
                        return (
                          <div
                            key={project.id}
                            className="flex flex-col gap-0.5"
                          >
                            {renamingProjectId === project.id ? (
                              <FolderRenameRow
                                initial={project.name}
                                collapsed={projectCollapsed}
                                live={live}
                                attention={nestedAttention}
                                inputLabel="Project name"
                                onSubmit={(nextName) =>
                                  void submitProjectRename(
                                    project.id,
                                    project.name,
                                    nextName,
                                  )
                                }
                                onCancel={() => setRenamingProjectId(null)}
                              />
                            ) : (
                              <div
                                onContextMenu={(event) => {
                                  event.preventDefault();
                                  openProjectMenu(project, {
                                    x: event.clientX,
                                    y: event.clientY,
                                  });
                                }}
                                className={`group flex items-center gap-1.5 rounded border px-2.5 py-1.5 text-xs transition-colors ${
                                  activeProjectId === project.id
                                    ? "border-sidebar-selected-border bg-sidebar-selected text-fg shadow-sm"
                                    : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
                                }`}
                              >
                                <button
                                  type="button"
                                  onClick={() => void toggleProject(project)}
                                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                                  title={project.cwd}
                                >
                                  {projectCollapsed ? (
                                    <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
                                  ) : (
                                    <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
                                  )}
                                  <SidebarTabIcon icon={FolderCode} active={live} />
                                  <span className="min-w-0 flex-1 truncate font-medium">
                                    {project.name}
                                  </span>
                                  <ChatAttentionIndicator
                                    state={
                                      projectCollapsed ? nestedAttention : null
                                    }
                                  />
                                </button>
                                <button
                                  type="button"
                                  onClick={(event) =>
                                    openProjectMenu(project, {
                                      x: event.clientX,
                                      y: event.clientY,
                                    })
                                  }
                                  className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
                                  aria-label="Project actions"
                                  title="Project actions"
                                >
                                  <MoreHorizontal aria-hidden className="h-3 w-3" />
                                </button>
                              </div>
                            )}
                            {projectCollapsed ? null : (
                              <div className="ml-3 flex flex-col gap-0.5 border-l border-line pl-2">
                                {nestedMissions.map((mission) => (
                                  <MissionRow
                                    key={mission.id}
                                    mission={mission}
                                    selected={mission.id === currentMissionId}
                                    onClick={() => {
                                      setActiveProjectId(project.id);
                                      openMission(mission.id);
                                    }}
                                    onContextMenu={(anchor) =>
                                      openMissionMenu(mission, anchor)
                                    }
                                    renaming={renamingMissionId === mission.id}
                                    onRenameSubmit={(next) =>
                                      void submitMissionRename(mission.id, next)
                                    }
                                    onRenameCancel={() =>
                                      setRenamingMissionId(null)
                                    }
                                  />
                                ))}
                                {renderTabItems(nestedTabs, null, false)}
                                {nestedMissions.length === 0 &&
                                nestedTabs.length === 0 ? (
                                  <p className="px-2.5 py-1 text-xs text-fg-3">
                                    No chats or missions yet.
                                  </p>
                                ) : null}
                              </div>
                            )}
                          </div>
                        );
                      })
                    )}
                  </div>
                ) : null}
              </section>

              <section className="mt-5 flex shrink-0 flex-col">
                <CollapsibleSectionHeader
                  label="MISSION"
                  open={missionsOpen}
                  attention={missionsOpen ? null : missionAttention}
                  attentionWorkingLabel="Mission working"
                  onToggle={toggleMissions}
                  onPlus={() => {
                    setNewMissionProjectId(null);
                    setCreatingMission(true);
                  }}
                  plusTitle="Start mission"
                />
                {missionsOpen ? (
                  <div className="flex max-h-[38vh] flex-col gap-0.5 overflow-y-auto px-3 pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
                    {ungroupedMissions.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No live missions.
                      </p>
                    ) : (
                      ungroupedMissions.map((m) => (
                        <MissionRow
                          key={m.id}
                          mission={m}
                          selected={m.id === currentMissionId}
                          onClick={() => {
                            setActiveProjectId(null);
                            openMission(m.id);
                          }}
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
                    <DndContext
                      sensors={tabDragSensors}
                      collisionDetection={pointerWithin}
                      onDragStart={handleTabDragStart}
                      onDragOver={handleTabDragOver}
                      onDragEnd={handleTabDragEnd}
                      onDragCancel={clearTabDrag}
                    >
                    {creatingFolder ? (
                      <NewFolderRow
                        onSubmit={submitFolderCreate}
                        onCancel={() => setCreatingFolder(false)}
                      />
                    ) : null}
                    {folders.length === 0 &&
                    chatSectionTabItems.length === 0 &&
                    !creatingFolder ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No chats yet.
                      </p>
                    ) : (
                      <>
                        {folders.map((folder) => {
                          const items = chatSectionTabItems.filter(
                            (item) => item.layout.folderId === folder.id,
                          );
                          const folderAttention = rollupAttentionState(
                            items.map((item) => item.attention),
                          );
                          const folderLive = items.some((item) =>
                            chatTabIsLive(item.members),
                          );
                          const folderCollapsed = collapsedFolderIds.has(
                            folder.id,
                          );
                          const visuallyCollapsed =
                            folderCollapsed &&
                            collapsedFolderDropId !== folder.id;
                          return (
                            <div
                              key={folder.id}
                              className="flex flex-col gap-0.5"
                            >
                              {renamingFolderId === folder.id ? (
                                <FolderRenameRow
                                  initial={folder.name}
                                  collapsed={folderCollapsed}
                                  live={folderLive}
                                  attention={folderAttention}
                                  onSubmit={(nextName) =>
                                    void submitFolderRename(
                                      folder.id,
                                      folder.name,
                                      nextName,
                                    )
                                  }
                                  onCancel={() => setRenamingFolderId(null)}
                                />
                              ) : (
                                <CollapsedFolderDropRow
                                  folderId={folder.id}
                                  enabled={
                                    folderCollapsed && draggedTabId !== null
                                  }
                                  active={
                                    collapsedFolderDropId === folder.id
                                  }
                                  onContextMenu={(anchor) =>
                                    openFolderMenu(folder, anchor)
                                  }
                                >
                                  <button
                                    type="button"
                                    onClick={() => toggleFolder(folder.id)}
                                    className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                                  >
                                    {visuallyCollapsed ? (
                                      <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
                                    ) : (
                                      <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
                                    )}
                                    <SidebarTabIcon
                                      icon={Folder}
                                      active={folderLive}
                                    />
                                    <span className="min-w-0 flex-1 truncate font-medium">
                                      {folder.name}
                                    </span>
                                    <ChatAttentionIndicator
                                      state={
                                        visuallyCollapsed
                                          ? folderAttention
                                          : null
                                      }
                                    />
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
                                </CollapsedFolderDropRow>
                              )}
                              {visuallyCollapsed ? null : (
                                <div className="ml-3 flex flex-col gap-0.5 border-l border-line pl-2">
                                  {renderTabItems(items, folder.id)}
                                </div>
                              )}
                            </div>
                          );
                        })}
                        {renderTabItems(
                          chatSectionTabItems.filter(
                            (item) => item.layout.folderId === null,
                          ),
                          null,
                        )}
                      </>
                    )}
                    <DragOverlay dropAnimation={null}>
                      {draggedTabItem ? (
                        <div className="shadow-[0_8px_24px_rgba(0,0,0,0.45)]">
                          <ChatTabGroup
                            layout={draggedTabItem.layout}
                            members={draggedTabItem.members}
                            active={draggedTabItem.members.some(
                              (member) =>
                                member.session_id === currentChatSessionId,
                            )}
                            attention={draggedTabItem.attention}
                            onActivate={() => undefined}
                            onContextMenu={() => undefined}
                          />
                        </div>
                      ) : null}
                    </DragOverlay>
                    </DndContext>
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
        project={newMissionProject}
        onClose={() => {
          setCreatingMission(false);
          setNewMissionProjectId(null);
        }}
        onStarted={(mission) => {
          setCreatingMission(false);
          setNewMissionProjectId(null);
          setActiveProjectId(mission.project_id);
          void refreshMissions();
          navigate(`/missions/${mission.id}`);
        }}
      />

      <StartChatModal
        open={creatingChat}
        project={newChatProject}
        onClose={() => {
          setCreatingChat(false);
          setNewChatFolderId(null);
          setNewChatProjectId(null);
        }}
        onStarted={(spawned) => {
          setCreatingChat(false);
          const targetFolderId = newChatFolderId;
          const targetProjectId = newChatProjectId;
          setNewChatFolderId(null);
          setNewChatProjectId(null);
          if (targetProjectId) {
            setActiveProjectId(targetProjectId);
            void refreshDirectSessions().finally(() => {
              navigate(`/chats/${spawned.id}`, {
                state: { sessionStatus: "running" },
              });
            });
            return;
          }
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
          archiveLabel={chatTabArchiveLabel(chatTabMenu.layout)}
          onClose={closeChatTabMenu}
          onPin={() => {
            void setChatTabPin(
              chatTabMenu.members,
              !chatTabMenu.members.every((member) => member.pinned),
            );
            closeChatTabMenu();
          }}
          onRename={() => {
            setRenamingChatTabId(chatTabMenu.layout.id);
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
          onNewChat={() => {
            if (collapsedFolderIds.has(folderMenu.id)) {
              toggleFolder(folderMenu.id);
            }
            handleNewFolderChat(folderMenu.id);
            closeFolderMenu();
          }}
          onRename={() => {
            setRenamingFolderId(folderMenu.id);
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

      {projectMenu ? (
        <ProjectContextMenu
          anchorX={projectMenu.x}
          anchorY={projectMenu.y}
          onClose={closeProjectMenu}
          onNewChat={() => {
            if (collapsedProjectIds.has(projectMenu.project.id)) {
              toggleProject(projectMenu.project);
            }
            handleNewProjectChat(projectMenu.project.id);
            closeProjectMenu();
          }}
          onNewMission={() => {
            if (collapsedProjectIds.has(projectMenu.project.id)) {
              toggleProject(projectMenu.project);
            }
            handleNewProjectMission(projectMenu.project.id);
            closeProjectMenu();
          }}
          onRename={() => {
            setRenamingProjectId(projectMenu.project.id);
            closeProjectMenu();
          }}
          onDelete={() => {
            setProjectDeleteConfirm(projectMenu.project);
            closeProjectMenu();
          }}
        />
      ) : null}

      <ConfirmDialog
        open={projectDeleteConfirm !== null}
        title={`Delete project "${projectDeleteConfirm?.name ?? ""}"?`}
        body="Chats and missions in this project will move back to the ungrouped sections. The on-disk directory and all of its files will remain untouched."
        confirmLabel="Delete project"
        busyLabel="Deleting…"
        busy={deletingProject}
        onConfirm={() => {
          if (projectDeleteConfirm) void deleteProject(projectDeleteConfirm);
        }}
        onCancel={() => setProjectDeleteConfirm(null)}
      />

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
  attentionWorkingLabel,
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
  attentionWorkingLabel?: string;
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
          <ChatAttentionIndicator
            state={attention}
            workingLabel={attentionWorkingLabel}
          />
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

function SortableChatTab({
  tabId,
  folderId,
  disabled,
  children,
}: {
  tabId: string;
  folderId: string | null;
  disabled: boolean;
  children: ReactNode;
}) {
  const {
    listeners,
    setNodeRef,
  } = useSortable({
    id: tabDndId(tabId),
    disabled,
    data: { kind: "tab", tabId, folderId } satisfies ChatTabDndData,
  });

  return (
    <div ref={setNodeRef} style={{ touchAction: "none" }} {...listeners}>
      {children}
    </div>
  );
}

function CollapsedFolderDropRow({
  folderId,
  enabled,
  active,
  onContextMenu,
  children,
}: {
  folderId: string;
  enabled: boolean;
  active: boolean;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  children: ReactNode;
}) {
  const { setNodeRef } = useDroppable({
    id: collapsedFolderDndId(folderId),
    disabled: !enabled,
    data: {
      kind: "collapsed-folder",
      folderId,
    } satisfies ChatTabDndData,
  });

  return (
    <div
      ref={setNodeRef}
      onContextMenu={(event) => {
        event.preventDefault();
        onContextMenu({ x: event.clientX, y: event.clientY });
      }}
      className={`group flex items-center gap-1.5 rounded border px-2.5 py-1.5 text-xs transition-colors ${
        active
          ? "border-accent bg-accent/10 text-fg"
          : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
      }`}
    >
      {children}
    </div>
  );
}

function EmptyTabDropArea({
  id,
  enabled,
  folderId,
  markerKey,
  active,
  label,
}: {
  id: string;
  enabled: boolean;
  folderId: string | null;
  markerKey: string;
  active: boolean;
  label: string | null;
}) {
  const { setNodeRef } = useDroppable({
    id,
    disabled: !enabled,
    data: {
      kind: "position",
      folderId,
      index: 0,
      markerKey,
    } satisfies ChatTabDndData,
  });

  return (
    <div ref={setNodeRef} className="relative min-h-7 shrink-0">
      {active ? (
        <>
          <span className="absolute left-0 top-1 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent" />
          <span className="absolute inset-x-0 top-1 h-0.5 -translate-y-1/2 rounded-full bg-accent" />
        </>
      ) : null}
      {label ? (
        <p className="px-2.5 py-1.5 text-xs text-fg-3">{label}</p>
      ) : null}
    </div>
  );
}

function TabDropDivider({
  id,
  enabled,
  folderId,
  index,
  markerKey,
  active,
}: {
  id: string;
  enabled: boolean;
  folderId: string | null;
  index: number;
  markerKey: string;
  active: boolean;
}) {
  const { setNodeRef } = useDroppable({
    id,
    disabled: !enabled,
    data: {
      kind: "position",
      folderId,
      index,
      markerKey,
    } satisfies ChatTabDndData,
  });

  return (
    <div ref={setNodeRef} className="relative z-20 -my-1 h-2 shrink-0">
      {active ? (
        <>
          <span className="absolute left-0 top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent" />
          <span className="absolute inset-x-0 top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-accent" />
        </>
      ) : null}
    </div>
  );
}

export function NewFolderRow({
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
      <SidebarTabIcon icon={Folder} active={false} />
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

function FolderRenameRow({
  initial,
  collapsed,
  live,
  attention,
  inputLabel = "Folder name",
  onSubmit,
  onCancel,
}: {
  initial: string;
  collapsed: boolean;
  live: boolean;
  attention: ChatAttentionState;
  inputLabel?: string;
  onSubmit: (name: string) => void;
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
      onContextMenu={(event) => event.stopPropagation()}
      className="flex items-center gap-1.5 rounded border border-sidebar-selected-border bg-sidebar-selected px-2.5 py-1.5 text-xs text-fg shadow-sm"
    >
      {collapsed ? (
        <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
      ) : (
        <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
      )}
      <SidebarTabIcon
        icon={Folder}
        active={live}
      />
      <input
        ref={inputRef}
        value={draft}
        aria-label={inputLabel}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onSubmit(draft.trim());
          } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (draft.trim() === initial.trim()) onCancel();
          else onSubmit(draft.trim());
        }}
        className="min-w-0 flex-1 bg-transparent text-xs font-medium text-fg outline-none"
      />
      <ChatAttentionIndicator state={collapsed ? attention : null} />
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

export function MissionRow({
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
    <SidebarTabRow
      selected={selected}
      label={mission.title}
      icon={Flag}
      iconActive={mission.all_sessions_live}
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={tooltip}
      attention={missionAttentionState(
        mission.any_session_live,
        mission.activity,
      )}
      attentionWorkingLabel="Mission working"
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
    <SidebarTabRow
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
}: {
  pinned: boolean;
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onOpenInNewWindow?: () => void;
  onArchive: () => void;
  renameLabel?: string;
  archiveLabel?: string;
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
      {onOpenInNewWindow ? (
        <ContextMenuItem
          icon={AppWindow}
          label="Open in New Window"
          onClick={onOpenInNewWindow}
        />
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

function ProjectContextMenu({
  anchorX,
  anchorY,
  onClose,
  onNewChat,
  onNewMission,
  onRename,
  onDelete,
}: {
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onNewChat: () => void;
  onNewMission: () => void;
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
      style={{ position: "fixed", left: pos.x, top: pos.y, width: 200 }}
      className="z-50 flex flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
    >
      <ContextMenuItem
        icon={MessageSquarePlus}
        label="New chat in project"
        onClick={onNewChat}
      />
      <ContextMenuItem
        icon={Flag}
        label="New mission in project"
        onClick={onNewMission}
      />
      <div className="my-1 h-px bg-line" />
      <ContextMenuItem icon={SquarePen} label="Rename project" onClick={onRename} />
      <div className="my-1 h-px bg-line" />
      <ContextMenuItem icon={Trash2} label="Delete project" onClick={onDelete} danger />
    </div>
  );
}

function FolderContextMenu({
  anchorX,
  anchorY,
  onClose,
  onNewChat,
  onRename,
  onDelete,
}: {
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onNewChat: () => void;
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
      <ContextMenuItem
        icon={MessageSquarePlus}
        label="New chat in folder"
        onClick={onNewChat}
      />
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
