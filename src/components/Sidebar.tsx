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
  ChevronDown,
  ChevronRight,
  Plus,
  Search,
  Terminal,
  Users,
} from "lucide-react";

import { api } from "../lib/api";
import {
  clearActiveSession,
  setActiveSession,
} from "../lib/activeSessions";
import type {
  AppendedEvent,
  MissionSummary,
  RunnerActivityEvent,
  RunnerWithActivity,
} from "../lib/types";
import { StartMissionModal } from "./StartMissionModal";

interface ActiveRunner {
  id: string;
  handle: string;
  active_missions: number;
  direct_session_id: string;
}

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

export function Sidebar() {
  const navigate = useNavigate();
  const location = useLocation();

  // Width + resize state.
  const [width, setWidth] = useState<number>(getStoredWidth);

  // Runtime state.
  const [missions, setMissions] = useState<MissionSummary[]>([]);
  const [active, setActive] = useState<ActiveRunner[]>([]);

  // Section toggles, persisted so users don't have to re-expand each visit.
  const [missionsOpen, setMissionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_MISSION_OPEN, true),
  );
  const [sessionsOpen, setSessionsOpen] = useState<boolean>(() =>
    getStoredFlag(STORAGE_SESSION_OPEN, true),
  );

  const [creatingMission, setCreatingMission] = useState(false);

  // Identify the currently-open runtime so we can highlight the matching
  // sidebar row. `useMatch` returns null when the URL doesn't match.
  const missionMatch = useMatch("/missions/:id");
  const currentMissionId = missionMatch?.params.id ?? null;
  const chatMatch = useMatch("/runners/:handle/chat");
  const currentChatHandle = chatMatch?.params.handle ?? null;

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

  // Direct-chat list: same listener pattern the previous Sidebar used.
  useEffect(() => {
    void api.runner.listWithActivity().then((rows: RunnerWithActivity[]) => {
      setActive(
        rows
          .filter((r) => r.direct_session_id !== null)
          .map((r) => ({
            id: r.id,
            handle: r.handle,
            active_missions: r.active_missions,
            direct_session_id: r.direct_session_id as string,
          })),
      );
      for (const r of rows) {
        if (r.direct_session_id) {
          setActiveSession(r.handle, r.direct_session_id);
        } else {
          clearActiveSession(r.handle);
        }
      }
    });
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<RunnerActivityEvent>("runner/activity", (event) => {
      const ev = event.payload;
      if (ev.direct_session_id) {
        setActiveSession(ev.handle, ev.direct_session_id);
      } else {
        clearActiveSession(ev.handle);
      }
      setActive((prev) => {
        const without = prev.filter((r) => r.id !== ev.runner_id);
        if (!ev.direct_session_id) return without;
        return [
          ...without,
          {
            id: ev.runner_id,
            handle: ev.handle,
            active_missions: ev.active_missions,
            direct_session_id: ev.direct_session_id,
          },
        ].sort((a, b) => a.handle.localeCompare(b.handle));
      });
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
  }, []);

  // Direct-chat list is always shown unfiltered. Filtering / search lives
  // on the WORKSPACE › search affordance at the top of the sidebar.
  const filteredDirect = active;

  const openMission = useCallback(
    (id: string) => {
      navigate(`/missions/${id}`);
    },
    [navigate],
  );

  const openDirectChat = useCallback(
    (handle: string, sessionId: string) => {
      const target = `/runners/${handle}/chat`;
      setActiveSession(handle, sessionId);
      navigate(target, {
        state: { sessionId },
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
        style={{ width }}
        className="relative flex h-full shrink-0 select-none flex-col overflow-hidden border-r border-line bg-raised"
      >
        <div data-tauri-drag-region className="h-7" />

        <div className="flex items-center gap-2 px-5 pb-5 pt-1">
          <BrandMark />
          <span className="text-base font-semibold tracking-tight text-fg">
            Runner
          </span>
        </div>

        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto pb-4">
          <SectionHeader>WORKSPACE</SectionHeader>
          <nav className="flex flex-col gap-0.5 px-3 pb-1">
            <DisabledNavRow
              icon={Search}
              label="search"
              hint="Coming soon"
            />
            <NavRow icon={Terminal} to="/runners" label="runner" />
            <NavRow icon={Users} to="/crews" label="crew" />
          </nav>

          <div className="h-5" />

          <CollapsibleSectionHeader
            label="MISSION"
            count={missions.length}
            open={missionsOpen}
            onToggle={toggleMissions}
            onPlus={() => setCreatingMission(true)}
            plusTitle="Start mission"
          />
          {missionsOpen ? (
            <div className="flex flex-col gap-0.5 px-3 pt-1">
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
                    title={`${m.crew_name || ""}${
                      m.pending_ask_count > 0
                        ? ` · ${m.pending_ask_count} pending`
                        : ""
                    }`}
                    pendingAsks={m.pending_ask_count}
                  />
                ))
              )}
            </div>
          ) : null}

          <div className="h-8" />

          <CollapsibleSectionHeader
            label="SESSION"
            count={active.length}
            open={sessionsOpen}
            onToggle={toggleSessions}
            onPlus={handleNewDirectChat}
            plusTitle="Start a direct chat"
          />
          {sessionsOpen ? (
            <div className="flex flex-col gap-0.5 px-3 pt-1">
              {filteredDirect.length === 0 ? (
                <p className="px-2.5 py-1 text-xs text-fg-3">
                  No live sessions.
                </p>
              ) : (
                filteredDirect.map((r) => (
                  <RuntimeRow
                    key={r.id}
                    selected={r.handle === currentChatHandle}
                    label={`@${r.handle} direct`}
                    mono
                    onClick={() => openDirectChat(r.handle, r.direct_session_id)}
                    title={`direct chat${
                      r.active_missions > 0
                        ? ` · ${r.active_missions} mission${
                            r.active_missions === 1 ? "" : "s"
                          }`
                        : ""
                    }`}
                  />
                ))
              )}
            </div>
          ) : null}
        </div>

        <div
          onMouseDown={handleResizeStart}
          title="Drag to resize"
          className="absolute right-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
        />
      </aside>

      <StartMissionModal
        open={creatingMission}
        onClose={() => setCreatingMission(false)}
        onStarted={(mission) => {
          setCreatingMission(false);
          void refreshMissions();
          navigate(`/missions/${mission.id}`);
        }}
      />
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

function DisabledNavRow({
  icon: Icon,
  label,
  hint,
}: {
  icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  label: string;
  hint?: string;
}) {
  return (
    <span
      title={hint}
      aria-disabled="true"
      className="flex cursor-not-allowed items-center gap-2 rounded px-2.5 py-1.5 text-sm text-fg-3"
    >
      <Icon aria-hidden className="h-3 w-3 text-fg-3" />
      <span>{label}</span>
    </span>
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
  title,
  mono,
  pendingAsks,
}: {
  selected: boolean;
  label: string;
  onClick: () => void;
  title?: string;
  mono?: boolean;
  pendingAsks?: number;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className={`flex w-full cursor-pointer items-center gap-2 rounded px-2.5 py-1.5 text-left text-xs transition-colors ${
        selected
          ? "border border-line bg-bg text-fg"
          : "border border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      <span className="inline-flex h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
      <span className={`truncate flex-1 ${mono ? "font-mono" : ""}`}>
        {label}
      </span>
      {pendingAsks && pendingAsks > 0 ? (
        <span
          title="Awaiting human input"
          className="rounded bg-warn/20 px-1 py-px text-[9px] font-bold uppercase tracking-wide text-warn"
        >
          {pendingAsks}
        </span>
      ) : null}
    </button>
  );
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
