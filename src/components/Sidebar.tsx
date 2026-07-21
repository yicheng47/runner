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
  FolderCode,
  FolderMinus,
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
  type NodeRow,
  type ProjectRow,
} from "../lib/api";
import {
  markArchivingMission,
  markArchivingSession,
  unmarkArchivingMission,
  unmarkArchivingSession,
} from "../lib/archivingState";
import {
  pinnedSessionIds,
  shouldInheritPinOnAdd,
} from "../lib/groupPinning";
import {
  chatTabArchiveLabel,
  chatTabIsLive,
  isChatTabDropIndexAllowed,
  orderedChatTabIdsAfterDrop,
} from "../lib/chatTabs";
import { orderedRootNodeIdsAfterProjectDrop } from "../lib/sidebarDnd";
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
  focusPane,
  getPaneLayout,
  leafForSession,
  newChatTargetPane,
  removeArchivedSessionFromLayout,
  hydratePaneLayoutsFromDb,
  moveNode,
  setGroupNameForSession,
  useNavNodes,
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
import { setActiveProjectScope } from "../lib/projectScope";
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
const STORAGE_SESSION_OPEN = "runner.sidebar.session.open";
const SIDEBAR_NAVIGATE_EVENT = "runner:navigate-sidebar-page";
const SIDEBAR_NAVIGATION_HISTORY_LIMIT = 64;

// One dnd vocabulary for every draggable sidebar row: leaf positions are
// per-scope, project positions reorder root projects, and project containers
// accept tab/mission drops at the child scope's end.
type RowDropTarget = {
  dropKind: "leaf" | "project";
  parentId: string | null;
  index: number;
  markerKey: string;
};

type SortableDisabled = NonNullable<
  Parameters<typeof useSortable>[0]["disabled"]
>;

type NavDndData =
  | {
      kind: "row";
      nodeId: string;
      parentId: string | null;
    }
  | ({ kind: "position" } & RowDropTarget)
  | {
      kind: "container";
      containerId: string;
    };

const rowDndId = (nodeId: string) => `nav-row:${nodeId}`;
const rowDropDndId = (parentId: string | null, markerKey: string) =>
  `nav-row-drop:${parentId ?? "root"}:${markerKey}`;
const containerDndId = (containerId: string) =>
  `nav-container-drop:${containerId}`;

/** A sidebar row resolved from a tree node: tab nodes join their pane
 *  layout + member sessions, mission nodes their running summary. Nodes
 *  whose content is missing (archived member rows mid-refresh,
 *  non-running missions) resolve to nothing and stay hidden. */
type NavRowModel =
  | {
      kind: "tab";
      node: NodeRow;
      layout: PaneLayout;
      members: DirectSessionEntry[];
      pinned: boolean;
      attention: ChatAttentionState;
    }
  | { kind: "mission"; node: NodeRow; mission: MissionSummary };

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
    pinned: boolean;
    /** The containing project when the row sits inside one — enables
     *  the explicit "Remove from project" menu action (drags out of a
     *  project are deliberately suppressed). */
    projectId: string | null;
    x: number;
    y: number;
  } | null>(null);
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
  // CHAT creation state. The `+` and empty-space context menus start a chat.
  const [creatingChat, setCreatingChat] = useState(false);
  const [newChatProjectId, setNewChatProjectId] = useState<string | null>(null);
  const [chatAddMenuOpen, setChatAddMenuOpen] = useState(false);
  const [renamingChatTabId, setRenamingChatTabId] = useState<string | null>(
    null,
  );
  const [chatCreateMenu, setChatCreateMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const [containerDropId, setContainerDropId] = useState<string | null>(null);
  const [draggedNodeId, setDraggedNodeId] = useState<string | null>(null);
  const [rowDropTarget, setRowDropTarget] = useState<RowDropTarget | null>(
    null,
  );
  const rowDropTargetRef = useRef<RowDropTarget | null>(null);

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

  // Durable nav state: the node tree drives every section; tab rows
  // join their pane layouts, mission rows their running summaries.
  const paneLayouts = usePaneLayouts();
  const navNodes = useNavNodes();

  // Sidebar rows per scope, in render order (the tree query already
  // sorts pinned-first within each parent, then by position). Root
  // scope excludes project nodes — those render as the PROJECT section.
  const rowsByParent = useMemo(() => {
    const sessionById = new Map(
      directSessions.map((session) => [session.session_id, session]),
    );
    const layoutById = new Map(
      paneLayouts.filter((layout) => layout.id).map((l) => [l.id, l]),
    );
    const missionById = new Map(missions.map((m) => [m.id, m]));
    const map = new Map<string | null, NavRowModel[]>();
    for (const node of navNodes) {
      let row: NavRowModel | null = null;
      if (node.type === "tab") {
        const layout = layoutById.get(node.id);
        const members = layout
          ? visibleSessionIds(layout.root)
              .map((id) => sessionById.get(id))
              .filter(
                (session): session is DirectSessionEntry =>
                  session !== undefined,
              )
          : [];
        if (layout && members.length > 0) {
          row = {
            kind: "tab",
            node,
            layout,
            members,
            pinned: node.pinned_position !== null,
            attention: tabAttentionState(
              members,
              directSessionActivity,
              layout.lastCompletedAt,
              layout.lastViewedAt,
            ),
          };
        }
      } else if (node.type === "mission") {
        const mission = node.ref_id ? missionById.get(node.ref_id) : undefined;
        if (mission) row = { kind: "mission", node, mission };
      }
      if (!row) continue;
      const list = map.get(node.parent_id);
      if (list) list.push(row);
      else map.set(node.parent_id, [row]);
    }
    return map;
  }, [directSessionActivity, directSessions, missions, navNodes, paneLayouts]);

  const scopeRows = useCallback(
    (parentId: string | null): NavRowModel[] => rowsByParent.get(parentId) ?? [],
    [rowsByParent],
  );

  const projectNodes = useMemo(
    () =>
      navNodes.filter(
        (node) => node.parent_id === null && node.type === "project",
      ),
    [navNodes],
  );
  const recentRows = scopeRows(null);
  const draggedNode = useMemo(
    () => navNodes.find((node) => node.id === draggedNodeId) ?? null,
    [draggedNodeId, navNodes],
  );

  const rowAttention = useCallback(
    (row: NavRowModel): ChatAttentionState => {
      if (row.kind === "tab") return row.attention;
      return missionAttentionState(
        row.mission.any_session_live,
        row.mission.activity,
      );
    },
    [],
  );

  function rowIsLive(row: NavRowModel): boolean {
    if (row.kind === "tab") return chatTabIsLive(row.members);
    return row.mission.any_session_live;
  }

  const recentAttention = useMemo(
    () => rollupAttentionState(recentRows.map(rowAttention)),
    [recentRows, rowAttention],
  );
  const projectAttention = useMemo(
    () =>
      rollupAttentionState(
        projectNodes.flatMap((node) =>
          scopeRows(node.id).map(rowAttention),
        ),
      ),
    [projectNodes, rowAttention, scopeRows],
  );
  const draggedRow = useMemo(() => {
    if (!draggedNodeId) return null;
    for (const rows of rowsByParent.values()) {
      const row = rows.find((candidate) => candidate.node.id === draggedNodeId);
      if (row) return row;
    }
    return null;
  }, [draggedNodeId, rowsByParent]);
  const activeProject = useMemo(
    () => projects.find((project) => project.id === activeProjectId) ?? null,
    [activeProjectId, projects],
  );
  const draggedProject = useMemo(
    () =>
      draggedNode?.type === "project"
        ? (projects.find((project) => project.id === draggedNode.ref_id) ?? null)
        : null,
    [draggedNode, projects],
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

  // Page-back/forward candidates in display order: project children
  // first (as the PROJECT section renders above), then the recent list.
  const sidebarNavigationEntries = useMemo<SidebarNavigationEntry[]>(() => {
    const entries: SidebarNavigationEntry[] = [];
    const pushRow = (row: NavRowModel) => {
      if (row.kind === "mission") {
        entries.push({ to: `/missions/${row.mission.id}` });
      } else if (row.kind === "tab") {
        const target = row.members[0];
        entries.push({
          to: `/chats/${target.session_id}`,
          state: { sessionStatus: target.status },
        });
      }
    };
    for (const node of projectNodes) {
      for (const row of scopeRows(node.id)) pushRow(row);
    }
    for (const row of recentRows) pushRow(row);
    return entries;
  }, [projectNodes, recentRows, scopeRows]);

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
      pinned: boolean,
      projectId: string | null,
      anchor: { x: number; y: number },
    ) => {
      setMissionMenu(null);
      setProjectMenu(null);
      setChatTabMenu({
        layout,
        members,
        pinned,
        projectId,
        x: anchor.x,
        y: anchor.y,
      });
    },
    [],
  );
  const closeChatTabMenu = useCallback(() => setChatTabMenu(null), []);

  const openMissionMenu = useCallback(
    (mission: MissionSummary, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setProjectMenu(null);
      setMissionMenu({ mission, x: anchor.x, y: anchor.y });
    },
    [],
  );
  const closeMissionMenu = useCallback(() => setMissionMenu(null), []);
  const openProjectMenu = useCallback(
    (project: ProjectRow, anchor: { x: number; y: number }) => {
      setChatTabMenu(null);
      setMissionMenu(null);
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
      // Deleting a project archives everything inside it — if the open
      // chat/mission lives there, bounce off before the surface breaks.
      const projectNode = navNodes.find(
        (node) => node.type === "project" && node.ref_id === project.id,
      );
      const currentWasDeleted =
        projectNode !== undefined &&
        scopeRows(projectNode.id).some(
          (row) =>
            (row.kind === "tab" &&
              row.members.some(
                (member) => member.session_id === currentChatSessionId,
              )) ||
            (row.kind === "mission" && row.mission.id === currentMissionId),
        );
      // Leave an affected route BEFORE the destructive call — children
      // archive one by one, so even a failed delete can have archived
      // the open chat/mission already.
      if (currentWasDeleted) navigate("/runners");
      try {
        await api.project.delete(project.id);
        setProjectDeleteConfirm(null);
        if (activeProjectId === project.id) setActiveProjectId(null);
      } catch (e) {
        console.error("sidebar: project_delete failed", e);
      } finally {
        // Refresh regardless of outcome — a partial failure may have
        // durably archived some children.
        await Promise.all([
          refreshProjects(),
          refreshMissions(),
          refreshDirectSessions(),
          hydratePaneLayoutsFromDb(),
        ]).catch((e: unknown) =>
          console.error("sidebar: post-delete refresh failed", e),
        );
        setDeletingProject(false);
      }
    },
    [
      activeProjectId,
      currentChatSessionId,
      currentMissionId,
      navNodes,
      navigate,
      refreshDirectSessions,
      refreshMissions,
      refreshProjects,
      scopeRows,
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

  // The explicit exits from a project — drags out of a project scope
  // are suppressed, so these menu actions are the sanctioned path.
  // The backend reconciles the tab node to root / reparents the
  // mission node and emits the layout invalidation.
  const removeChatTabFromProject = useCallback(
    async (members: DirectSessionEntry[]) => {
      try {
        await api.session.setProject(
          members.map((member) => member.session_id),
          null,
        );
        await refreshDirectSessions();
      } catch (e) {
        console.error("sidebar: remove chat from project failed", e);
      }
    },
    [refreshDirectSessions],
  );

  const removeMissionFromProject = useCallback(
    async (mission: MissionSummary) => {
      try {
        await api.mission.setProject(mission.id, null);
        await refreshMissions();
      } catch (e) {
        console.error("sidebar: remove mission from project failed", e);
      }
    },
    [refreshMissions],
  );

  const setChatTabPin = useCallback(
    async (tabId: string, nextPinned: boolean) => {
      try {
        // Pin state lives on the tab node; the backend writes the
        // members' legacy pinned_at flags through for non-sidebar
        // surfaces (tray sort).
        await api.node.setPinned(tabId, nextPinned);
        await refreshDirectSessions();
      } catch (e) {
        console.error("sidebar: node_set_pinned failed", e);
      }
    },
    [refreshDirectSessions],
  );

  const submitChatTabRename = useCallback(
    (sessionId: string, nextName: string) => {
      setRenamingChatTabId(null);
      setGroupNameForSession(sessionId, nextName);
    },
    [],
  );

  const openChatCreateMenu = useCallback(
    (anchor: { x: number; y: number }) => {
      setChatAddMenuOpen(false);
      setChatTabMenu(null);
      setMissionMenu(null);
      setProjectMenu(null);
      setChatCreateMenu(anchor);
    },
    [],
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
      void api.node
        .markViewed(
          tabId,
          members.map((member) => member.session_id),
        )
        .catch((error: unknown) =>
          console.error("sidebar: node_mark_viewed failed", error),
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
    setNewChatProjectId(null);
    setCreatingChat(true);
  }, []);

  const handleNewProjectChat = useCallback((projectId: string) => {
    setChatCreateMenu(null);
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

  const clearRowDrag = useCallback(() => {
    setDraggedNodeId(null);
    setContainerDropId(null);
    setRowDropTarget(null);
    rowDropTargetRef.current = null;
  }, []);

  // Draggable-row lookup + the pin-tier list of a destination scope.
  // Only visible rows participate in drop index math; hidden siblings
  // (non-running mission nodes, tabs whose rows are mid-refresh) are
  // appended behind the visible ordering when the move commits.
  const draggedRowFor = useCallback(
    (nodeId: string): NavRowModel | null => {
      for (const rows of rowsByParent.values()) {
        const row = rows.find((candidate) => candidate.node.id === nodeId);
        if (row) return row;
      }
      return null;
    },
    [rowsByParent],
  );

  const scopeOrderedRows = useCallback(
    (parentId: string | null) =>
      scopeRows(parentId).map((row) => ({
        id: row.node.id,
        pinned: row.node.pinned_position !== null,
      })),
    [scopeRows],
  );

  // Drag affordance rules: leaves (tabs, missions) drop at root or in
  // projects, with one exception: a node currently inside a project
  // only drops into project scopes. Leaving a project unbinds
  // cwd/scope, so it stays an explicit menu action rather than
  // something an errant drag can do silently.
  const canDropInScope = useCallback(
    (nodeId: string, parentId: string | null): boolean => {
      const dragged = navNodes.find((node) => node.id === nodeId);
      if (!dragged) return false;
      const draggedParent = dragged.parent_id
        ? navNodes.find((node) => node.id === dragged.parent_id)
        : undefined;
      const leavingProject = draggedParent?.type === "project";
      if (parentId === null) return !leavingProject;
      const parent = navNodes.find((node) => node.id === parentId);
      if (!parent) return false;
      if (leavingProject && parent.type !== "project") return false;
      if (parent.type === "project") {
        return dragged.type === "tab" || dragged.type === "mission";
      }
      return false;
    },
    [navNodes],
  );

  const commitRowDrop = useCallback(
    async (
      nodeId: string,
      parentId: string | null,
      requestedIndex: number,
    ) => {
      const dragged = navNodes.find((node) => node.id === nodeId);
      if (!dragged) {
        clearRowDrag();
        return;
      }
      if (dragged.type === "project") {
        if (parentId !== null) {
          clearRowDrag();
          return;
        }
        const orderedIds = orderedRootNodeIdsAfterProjectDrop(
          navNodes,
          nodeId,
          requestedIndex,
        );
        clearRowDrag();
        try {
          await moveNode(nodeId, null, orderedIds);
        } catch (error) {
          console.error("sidebar: move project failed", error);
        }
        return;
      }
      const orderedVisible = orderedChatTabIdsAfterDrop(
        scopeOrderedRows(parentId),
        nodeId,
        dragged.pinned_position !== null,
        requestedIndex,
      );
      // The backend validates the COMPLETE child set of the scope:
      // project nodes keep their lead positions at root, and hidden
      // siblings follow the visible ordering.
      const visibleSet = new Set(orderedVisible);
      const siblings = navNodes.filter(
        (node) => node.parent_id === parentId && node.id !== nodeId,
      );
      const projectsFirst =
        parentId === null
          ? siblings
              .filter((node) => node.type === "project")
              .map((node) => node.id)
          : [];
      const hidden = siblings
        .filter(
          (node) => !(parentId === null && node.type === "project"),
        )
        .map((node) => node.id)
        .filter((id) => !visibleSet.has(id));
      clearRowDrag();
      try {
        await moveNode(nodeId, parentId, [
          ...projectsFirst,
          ...orderedVisible,
          ...hidden,
        ]);
      } catch (error) {
        console.error("sidebar: move node failed", error);
      }
    },
    [clearRowDrag, navNodes, scopeOrderedRows],
  );

  const resolveRowDropTarget = useCallback(
    (event: DragOverEvent | DragEndEvent): RowDropTarget | null => {
      const activeData = event.active.data.current as NavDndData | undefined;
      const overData = event.over?.data.current as NavDndData | undefined;
      if (activeData?.kind !== "row" || !overData) return null;

      const activeNode = navNodes.find(
        (node) => node.id === activeData.nodeId,
      );
      if (!activeNode) return null;
      if (activeNode.type === "project") {
        const projectTargetAt = (originalIndex: number): RowDropTarget => {
          const index = projectNodes
            .slice(0, originalIndex)
            .filter((node) => node.id !== activeNode.id).length;
          return {
            dropKind: "project",
            parentId: null,
            index,
            markerKey:
              originalIndex < projectNodes.length
                ? `project-before-${projectNodes[originalIndex].id}`
                : "project-after-root",
          };
        };
        const targetAfterProject = (
          projectId: string | null,
        ): RowDropTarget | null => {
          if (!projectId) return null;
          const projectIndex = projectNodes.findIndex(
            (node) => node.id === projectId,
          );
          return projectIndex < 0 ? null : projectTargetAt(projectIndex + 1);
        };

        if (overData.kind === "position") {
          if (overData.dropKind === "project") {
            return {
              dropKind: "project",
              parentId: null,
              index: overData.index,
              markerKey: overData.markerKey,
            };
          }
          return targetAfterProject(overData.parentId);
        }
        if (overData.kind !== "row") return null;
        const overIndex = projectNodes.findIndex(
          (node) => node.id === overData.nodeId,
        );
        if (overIndex < 0) {
          return targetAfterProject(overData.parentId);
        }
        if (!event.over) return null;

        const activeRect =
          event.active.rect.current.translated ??
          event.active.rect.current.initial;
        const after = activeRect
          ? activeRect.top + activeRect.height / 2 >=
            event.over.rect.top + event.over.rect.height / 2
          : false;
        const originalIndex = overIndex + (after ? 1 : 0);
        return projectTargetAt(originalIndex);
      }

      const dragged = draggedRowFor(activeData.nodeId);
      if (!dragged) return null;
      const draggedPinned = dragged.node.pinned_position !== null;

      if (overData.kind === "position") {
        if (overData.dropKind !== "leaf") return null;
        const allowed =
          canDropInScope(dragged.node.id, overData.parentId) &&
          isChatTabDropIndexAllowed(
            scopeOrderedRows(overData.parentId),
            dragged.node.id,
            draggedPinned,
            overData.index,
          );
        return allowed
          ? {
              dropKind: "leaf",
              parentId: overData.parentId,
              index: overData.index,
              markerKey: overData.markerKey,
            }
          : null;
      }

      if (overData.kind !== "row") return null;
      if (!canDropInScope(dragged.node.id, overData.parentId)) return null;
      const targetRows = scopeRows(overData.parentId);
      const overIndex = targetRows.findIndex(
        (row) => row.node.id === overData.nodeId,
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
      const index = targetRows
        .slice(0, originalIndex)
        .filter((row) => row.node.id !== dragged.node.id).length;
      if (
        !isChatTabDropIndexAllowed(
          scopeOrderedRows(overData.parentId),
          dragged.node.id,
          draggedPinned,
          index,
        )
      ) {
        return null;
      }

      return {
        dropKind: "leaf",
        parentId: overData.parentId,
        index,
        markerKey:
          originalIndex < targetRows.length
            ? `before-${targetRows[originalIndex].node.id}`
            : `after-${overData.parentId ?? "root"}`,
      };
    },
    [
      canDropInScope,
      draggedRowFor,
      navNodes,
      projectNodes,
      scopeOrderedRows,
      scopeRows,
    ],
  );

  const resolveContainerDropTarget = useCallback(
    (nodeId: string, containerId: string): RowDropTarget | null => {
      if (!canDropInScope(nodeId, containerId)) return null;
      const dragged = draggedRowFor(nodeId);
      if (!dragged) return null;
      const orderedIds = orderedChatTabIdsAfterDrop(
        scopeOrderedRows(containerId),
        nodeId,
        dragged.node.pinned_position !== null,
        Number.MAX_SAFE_INTEGER,
      );
      const index = orderedIds.indexOf(nodeId);
      const remaining = scopeRows(containerId).filter(
        (row) => row.node.id !== nodeId,
      );
      if (index < 0) return null;
      return {
        dropKind: "leaf",
        parentId: containerId,
        index,
        markerKey:
          index < remaining.length
            ? `before-${remaining[index].node.id}`
            : `after-${containerId}`,
      };
    },
    [canDropInScope, draggedRowFor, scopeOrderedRows, scopeRows],
  );

  const handleRowDragStart = useCallback((event: DragStartEvent) => {
    const data = event.active.data.current as NavDndData | undefined;
    if (data?.kind !== "row") return;
    setDraggedNodeId(data.nodeId);
    setContainerDropId(null);
    setRowDropTarget(null);
    rowDropTargetRef.current = null;
  }, []);

  const handleRowDragOver = useCallback(
    (event: DragOverEvent) => {
      const activeData = event.active.data.current as NavDndData | undefined;
      const overData = event.over?.data.current as NavDndData | undefined;
      if (activeData?.kind === "row" && overData?.kind === "container") {
        const target = resolveContainerDropTarget(
          activeData.nodeId,
          overData.containerId,
        );
        setContainerDropId(target ? overData.containerId : null);
        setRowDropTarget(target);
        rowDropTargetRef.current = target;
        return;
      }

      setContainerDropId((containerId) => {
        if (
          containerId &&
          (overData?.kind === "position" || overData?.kind === "row") &&
          overData.parentId === containerId
        ) {
          return containerId;
        }
        return null;
      });
      const target = resolveRowDropTarget(event);
      setRowDropTarget(target);
      rowDropTargetRef.current = target;
    },
    [resolveContainerDropTarget, resolveRowDropTarget],
  );

  const handleRowDragEnd = useCallback(
    (event: DragEndEvent) => {
      const activeData = event.active.data.current as NavDndData | undefined;
      const overData = event.over?.data.current as NavDndData | undefined;
      if (activeData?.kind !== "row") {
        clearRowDrag();
        return;
      }
      if (overData?.kind === "container") {
        if (canDropInScope(activeData.nodeId, overData.containerId)) {
          void commitRowDrop(
            activeData.nodeId,
            overData.containerId,
            Number.MAX_SAFE_INTEGER,
          );
        } else {
          clearRowDrag();
        }
        return;
      }
      const target = resolveRowDropTarget(event) ?? rowDropTargetRef.current;
      if (target) {
        void commitRowDrop(activeData.nodeId, target.parentId, target.index);
      } else {
        clearRowDrag();
      }
    },
    [canDropInScope, clearRowDrag, commitRowDrop, resolveRowDropTarget],
  );

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

  // Every leaf row (tab, mission) drags through the same
  // reparent/reposition op.
  const renderLeafRow = (
    row: NavRowModel,
    projectId: string | null,
  ): ReactNode => {
    const nodeId = row.node.id;
    if (row.kind === "mission") {
      return (
        <MissionRow
          mission={row.mission}
          selected={row.mission.id === currentMissionId}
          onClick={() => {
            setActiveProjectId(projectId);
            openMission(row.mission.id);
          }}
          onContextMenu={(anchor) => openMissionMenu(row.mission, anchor)}
          renaming={renamingMissionId === row.mission.id}
          onRenameSubmit={(next) =>
            void submitMissionRename(row.mission.id, next)
          }
          onRenameCancel={() => setRenamingMissionId(null)}
        />
      );
    }
    const active = row.members.some(
      (member) => member.session_id === currentChatSessionId,
    );
    return (
      <ChatTabGroup
        layout={row.layout}
        members={row.members}
        active={active}
        pinned={row.pinned}
        attention={row.attention}
        onActivate={(entry) => {
          setActiveProjectId(projectId);
          activateTabChat(nodeId, row.members, entry);
        }}
        onContextMenu={(anchor) =>
          openChatTabMenu(row.layout, row.members, row.pinned, projectId, anchor)
        }
        dragging={draggedNodeId === nodeId}
        renaming={renamingChatTabId === nodeId}
        onRenameSubmit={(nextName) =>
          submitChatTabRename(row.members[0].session_id, nextName)
        }
        onRenameCancel={() => setRenamingChatTabId(null)}
      />
    );
  };

  const renderRowDropDivider = (
    parentId: string | null,
    rows: NavRowModel[],
    originalIndex: number,
    key: string,
  ) => {
    const dragged = draggedNodeId ? draggedRowFor(draggedNodeId) : null;
    const index = rows
      .slice(0, originalIndex)
      .filter((row) => row.node.id !== draggedNodeId).length;
    const enabled = Boolean(
      dragged &&
        canDropInScope(dragged.node.id, parentId) &&
        isChatTabDropIndexAllowed(
          rows.map((row) => ({
            id: row.node.id,
            pinned: row.node.pinned_position !== null,
          })),
          dragged.node.id,
          dragged.node.pinned_position !== null,
          index,
        ),
    );
    return (
      <RowDropDivider
        key={key}
        id={rowDropDndId(parentId, key)}
        enabled={enabled}
        dropKind="leaf"
        parentId={parentId}
        index={index}
        markerKey={key}
        active={
          rowDropTarget?.dropKind === "leaf" &&
          rowDropTarget.parentId === parentId &&
          rowDropTarget.index === index &&
          rowDropTarget.markerKey === key
        }
      />
    );
  };

  const renderProjectDropDivider = (
    originalIndex: number,
    key: string,
  ) => {
    const index = projectNodes
      .slice(0, originalIndex)
      .filter((node) => node.id !== draggedNodeId).length;
    return (
      <RowDropDivider
        key={key}
        id={rowDropDndId(null, key)}
        enabled={draggedNode?.type === "project"}
        dropKind="project"
        parentId={null}
        index={index}
        markerKey={key}
        active={
          rowDropTarget?.dropKind === "project" &&
          rowDropTarget.index === index &&
          rowDropTarget.markerKey === key
        }
      />
    );
  };

  const renderNavRow = (
    row: NavRowModel,
    parentId: string | null,
    projectId: string | null,
  ): ReactNode => {
    return (
      <SortableNavRow
        key={row.node.id}
        nodeId={row.node.id}
        parentId={parentId}
        disabled={
          renamingChatTabId === row.node.id ||
          (row.kind === "mission" && renamingMissionId === row.mission.id)
        }
      >
        {renderLeafRow(row, projectId)}
      </SortableNavRow>
    );
  };

  const renderScopeRows = (
    rows: NavRowModel[],
    parentId: string | null,
    projectId: string | null,
    emptyLabel: string | null = null,
  ) => {
    if (rows.length === 0) {
      const markerKey = `after-${parentId ?? "root"}`;
      return (
        <SortableContext items={[]} strategy={verticalListSortingStrategy}>
          <EmptyRowDropArea
            id={rowDropDndId(parentId, markerKey)}
            enabled={
              draggedNodeId !== null &&
              canDropInScope(draggedNodeId, parentId)
            }
            dropKind="leaf"
            parentId={parentId}
            markerKey={markerKey}
            active={
              rowDropTarget?.dropKind === "leaf" &&
              rowDropTarget.parentId === parentId &&
              rowDropTarget.index === 0 &&
              rowDropTarget.markerKey === markerKey
            }
            label={emptyLabel}
          />
        </SortableContext>
      );
    }

    return (
      <SortableContext
        items={rows.map((row) => rowDndId(row.node.id))}
        strategy={verticalListSortingStrategy}
      >
        {rows.map((row, originalIndex) => (
          <Fragment key={row.node.id}>
            {renderRowDropDivider(
              parentId,
              rows,
              originalIndex,
              `before-${row.node.id}`,
            )}
            {renderNavRow(row, parentId, projectId)}
          </Fragment>
        ))}
        {renderRowDropDivider(
          parentId,
          rows,
          rows.length,
          `after-${parentId ?? "root"}`,
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

            <div className="flex min-h-0 flex-1 flex-col overflow-y-auto pb-3">
              <DndContext
                sensors={tabDragSensors}
                collisionDetection={pointerWithin}
                onDragStart={handleRowDragStart}
                onDragOver={handleRowDragOver}
                onDragEnd={handleRowDragEnd}
                onDragCancel={clearRowDrag}
              >
              <section className="flex shrink-0 flex-col">
                <CollapsibleSectionHeader
                  label="PROJECTS"
                  open={projectsOpen}
                  attention={projectsOpen ? null : projectAttention}
                  onToggle={toggleProjects}
                  onPlus={() => void addProject()}
                  plusTitle="Add project"
                />
                {projectsOpen ? (
                  <div className="flex flex-col gap-0.5 px-3 pt-1">
                    {projectNodes.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No projects yet.
                      </p>
                    ) : (
                      <SortableContext
                        items={projectNodes.map((node) => rowDndId(node.id))}
                        strategy={verticalListSortingStrategy}
                      >
                        {projectNodes.map((node, originalIndex) => {
                          const project = projects.find(
                            (candidate) => candidate.id === node.ref_id,
                          );
                          if (!project) return null;
                          const nestedRows = scopeRows(node.id);
                          const nestedAttention = rollupAttentionState(
                            nestedRows.map(rowAttention),
                          );
                          const live = nestedRows.some(rowIsLive);
                          const projectCollapsed = collapsedProjectIds.has(
                            project.id,
                          );
                          return (
                            <Fragment key={node.id}>
                              {renderProjectDropDivider(
                                originalIndex,
                                `project-before-${node.id}`,
                              )}
                              <div className="flex flex-col gap-0.5">
                                <SortableNavRow
                                  nodeId={node.id}
                                  parentId={null}
                                  disabled={{
                                    draggable:
                                      renamingProjectId === project.id,
                                    droppable:
                                      draggedNode?.type !== "project",
                                  }}
                                >
                                  {renamingProjectId === project.id ? (
                                    <ProjectRenameRow
                                      initial={project.name}
                                      collapsed={projectCollapsed}
                                      live={live}
                                      attention={nestedAttention}
                                      onSubmit={(nextName) =>
                                        void submitProjectRename(
                                          project.id,
                                          project.name,
                                          nextName,
                                        )
                                      }
                                      onCancel={() =>
                                        setRenamingProjectId(null)
                                      }
                                    />
                                  ) : (
                                    <ContainerDropRow
                                      containerId={node.id}
                                      enabled={
                                        draggedNodeId !== null &&
                                        canDropInScope(draggedNodeId, node.id)
                                      }
                                      active={containerDropId === node.id}
                                      selected={
                                        activeProjectId === project.id
                                      }
                                      onContextMenu={(anchor) =>
                                        openProjectMenu(project, anchor)
                                      }
                                    >
                                      <button
                                        type="button"
                                        onClick={() =>
                                          void toggleProject(project)
                                        }
                                        className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                                        title={project.cwd}
                                      >
                                        {projectCollapsed ? (
                                          <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
                                        ) : (
                                          <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
                                        )}
                                        <SidebarTabIcon
                                          icon={FolderCode}
                                          active={live}
                                        />
                                        <span className="min-w-0 flex-1 truncate font-medium">
                                          {project.name}
                                        </span>
                                        <ChatAttentionIndicator
                                          state={
                                            projectCollapsed
                                              ? nestedAttention
                                              : null
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
                                    </ContainerDropRow>
                                  )}
                                </SortableNavRow>
                                {projectCollapsed ? null : (
                                  <div className="ml-3 flex flex-col gap-0.5 border-l border-line pl-2">
                                    {nestedRows.length === 0 &&
                                    draggedNodeId === null ? (
                                      <p className="px-2.5 py-1 text-xs text-fg-3">
                                        No chats or missions yet.
                                      </p>
                                    ) : (
                                      renderScopeRows(
                                        nestedRows,
                                        node.id,
                                        project.id,
                                      )
                                    )}
                                  </div>
                                )}
                              </div>
                            </Fragment>
                          );
                        })}
                        {renderProjectDropDivider(
                          projectNodes.length,
                          "project-after-root",
                        )}
                      </SortableContext>
                    )}
                  </div>
                ) : null}
              </section>

              <section className="mt-5 flex flex-1 flex-col">
                <CollapsibleSectionHeader
                  label="CHATS & MISSIONS"
                  open={sessionsOpen}
                  attention={sessionsOpen ? null : recentAttention}
                  onToggle={toggleSessions}
                  onPlus={() => {
                    setChatCreateMenu(null);
                    setChatAddMenuOpen((open) => !open);
                  }}
                  plusTitle="Add chat or mission"
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
                        onClick={() => {
                          setChatAddMenuOpen(false);
                          setNewMissionProjectId(null);
                          setCreatingMission(true);
                        }}
                        className="flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-1.5 text-left text-[13px] text-fg hover:bg-line"
                      >
                        <Flag aria-hidden className="h-3.5 w-3.5" />
                        New mission
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
                    className="flex flex-1 flex-col gap-0.5 px-3 pt-1"
                  >
                    {recentRows.length === 0 ? (
                      <p className="px-2.5 py-1 text-xs text-fg-3">
                        No chats yet.
                      </p>
                    ) : (
                      renderScopeRows(recentRows, null, null)
                    )}
                  </div>
                ) : null}
              </section>
              <DragOverlay dropAnimation={null}>
                {draggedRow && draggedRow.kind === "tab" ? (
                  <div className="shadow-[0_8px_24px_rgba(0,0,0,0.45)]">
                    <ChatTabGroup
                      layout={draggedRow.layout}
                      members={draggedRow.members}
                      active={draggedRow.members.some(
                        (member) =>
                          member.session_id === currentChatSessionId,
                      )}
                      pinned={draggedRow.pinned}
                      attention={draggedRow.attention}
                      onActivate={() => undefined}
                      onContextMenu={() => undefined}
                    />
                  </div>
                ) : draggedRow && draggedRow.kind === "mission" ? (
                  <div className="shadow-[0_8px_24px_rgba(0,0,0,0.45)]">
                    <MissionRow
                      mission={draggedRow.mission}
                      selected={draggedRow.mission.id === currentMissionId}
                      onClick={() => undefined}
                      onContextMenu={() => undefined}
                      renaming={false}
                      onRenameSubmit={() => undefined}
                      onRenameCancel={() => undefined}
                    />
                  </div>
                ) : draggedProject ? (
                  <div className="shadow-[0_8px_24px_rgba(0,0,0,0.45)]">
                    <div className="flex items-center gap-1.5 rounded border border-sidebar-selected-border bg-sidebar-selected px-2.5 py-1.5 text-xs text-fg shadow-sm">
                      {collapsedProjectIds.has(draggedProject.id) ? (
                        <ChevronRight aria-hidden className="h-3 w-3 shrink-0" />
                      ) : (
                        <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
                      )}
                      <SidebarTabIcon icon={FolderCode} active={false} />
                      <span className="min-w-0 flex-1 truncate font-medium">
                        {draggedProject.name}
                      </span>
                    </div>
                  </div>
                ) : null}
              </DragOverlay>
              </DndContext>
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
          setNewChatProjectId(null);
        }}
        onStarted={(spawned) => {
          setCreatingChat(false);
          const targetProjectId = newChatProjectId;
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
          pinned={chatTabMenu.pinned}
          anchorX={chatTabMenu.x}
          anchorY={chatTabMenu.y}
          renameLabel="Rename tab"
          archiveLabel={chatTabArchiveLabel(chatTabMenu.layout)}
          onClose={closeChatTabMenu}
          onPin={() => {
            void setChatTabPin(chatTabMenu.layout.id, !chatTabMenu.pinned);
            closeChatTabMenu();
          }}
          onRename={() => {
            setRenamingChatTabId(chatTabMenu.layout.id);
            closeChatTabMenu();
          }}
          onRemoveFromProject={
            chatTabMenu.projectId !== null
              ? () => {
                  void removeChatTabFromProject(chatTabMenu.members);
                  closeChatTabMenu();
                }
              : undefined
          }
          onArchive={() => {
            void archiveChatTab(chatTabMenu.members);
            closeChatTabMenu();
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
        body="Deleting this project archives every chat and mission inside it (running ones are stopped first). Archived items appear in Settings → Archived. The on-disk directory and all of its files remain untouched."
        confirmLabel="Delete project"
        busyLabel="Archiving…"
        busy={deletingProject}
        onConfirm={() => {
          if (projectDeleteConfirm) void deleteProject(projectDeleteConfirm);
        }}
        onCancel={() => setProjectDeleteConfirm(null)}
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
          onRemoveFromProject={
            missionMenu.mission.project_id !== null
              ? () => {
                  void removeMissionFromProject(missionMenu.mission);
                  closeMissionMenu();
                }
              : undefined
          }
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
        className="flex min-w-0 items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3 hover:text-fg-2"
      >
        <span className="min-w-0 truncate">{label}</span>
        <ChevronDown
          aria-hidden
          className={`h-2.5 w-2.5 shrink-0 transition-transform ${
            open ? "" : "-rotate-90"
          }`}
        />
      </button>
      <div className="flex shrink-0 items-center gap-1.5">
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

function SortableNavRow({
  nodeId,
  parentId,
  disabled,
  children,
}: {
  nodeId: string;
  parentId: string | null;
  disabled: SortableDisabled;
  children: ReactNode;
}) {
  const {
    listeners,
    setNodeRef,
  } = useSortable({
    id: rowDndId(nodeId),
    disabled,
    data: { kind: "row", nodeId, parentId } satisfies NavDndData,
  });

  return (
    <div ref={setNodeRef} style={{ touchAction: "none" }} {...listeners}>
      {children}
    </div>
  );
}

/** Project header that doubles as a drop target. Dropping appends at
 *  the project's end. */
function ContainerDropRow({
  containerId,
  enabled,
  active,
  selected,
  onContextMenu,
  children,
}: {
  containerId: string;
  enabled: boolean;
  active: boolean;
  selected?: boolean;
  onContextMenu: (anchor: { x: number; y: number }) => void;
  children: ReactNode;
}) {
  const { setNodeRef } = useDroppable({
    id: containerDndId(containerId),
    disabled: !enabled,
    data: {
      kind: "container",
      containerId,
    } satisfies NavDndData,
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
          : selected
            ? "border-sidebar-selected-border bg-sidebar-selected text-fg shadow-sm"
            : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
      }`}
    >
      {children}
    </div>
  );
}

function EmptyRowDropArea({
  id,
  enabled,
  dropKind,
  parentId,
  markerKey,
  active,
  label,
}: {
  id: string;
  enabled: boolean;
  dropKind: RowDropTarget["dropKind"];
  parentId: string | null;
  markerKey: string;
  active: boolean;
  label: string | null;
}) {
  const { setNodeRef } = useDroppable({
    id,
    disabled: !enabled,
    data: {
      kind: "position",
      dropKind,
      parentId,
      index: 0,
      markerKey,
    } satisfies NavDndData,
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

function RowDropDivider({
  id,
  enabled,
  dropKind,
  parentId,
  index,
  markerKey,
  active,
}: {
  id: string;
  enabled: boolean;
  dropKind: RowDropTarget["dropKind"];
  parentId: string | null;
  index: number;
  markerKey: string;
  active: boolean;
}) {
  const { setNodeRef } = useDroppable({
    id,
    disabled: !enabled,
    data: {
      kind: "position",
      dropKind,
      parentId,
      index,
      markerKey,
    } satisfies NavDndData,
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

function ProjectRenameRow({
  initial,
  collapsed,
  live,
  attention,
  onSubmit,
  onCancel,
}: {
  initial: string;
  collapsed: boolean;
  live: boolean;
  attention: ChatAttentionState;
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
        icon={FolderCode}
        active={live}
      />
      <input
        ref={inputRef}
        value={draft}
        aria-label="Project name"
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
  onRemoveFromProject,
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
  /** Present only for rows inside a project — the explicit exit,
   *  since drags out of a project scope are suppressed. */
  onRemoveFromProject?: () => void;
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
      {onRemoveFromProject ? (
        <ContextMenuItem
          icon={FolderMinus}
          label="Remove from project"
          onClick={onRemoveFromProject}
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

function ChatCreateContextMenu({
  anchorX,
  anchorY,
  onClose,
  onNewChat,
}: {
  anchorX: number;
  anchorY: number;
  onClose: () => void;
  onNewChat: () => void;
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
