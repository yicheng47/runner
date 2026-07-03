// Mission workspace page (`/missions/:id`) — the live view the human
// works in once a mission is running. Three columns:
//   - left sidebar (the AppShell's persistent Sidebar)
//   - center: tab strip ("Feed" + one per runner pty) over either the
//     EventFeed + MissionInput dock, or one of the runner terminals
//   - right rail: RunnersRail with status dots + LEAD badge + open pty
//
// The rail's "open pty" link selects the runner's terminal tab.
// Terminals stay mounted and inactive panes are hidden with display:none, so
// each PTY's xterm scrollback survives tab-switching. The backend terminal
// snapshot covers bytes emitted before a pane first mounts.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  Flag,
  Info,
  MoreHorizontal,
  Pin,
  PinOff,
  PanelRight,
  PanelRightDashed,
  RotateCcw,
  SquarePen,
  Terminal,
  Users as UsersIcon,
  X,
} from "lucide-react";

import { api, type SessionRow } from "../lib/api";
import { useT, type TFn } from "../lib/i18n";
import { MOD_LABEL, isModKey, isOppositeMod } from "../lib/platform";
import type {
  AppendedEvent,
  Crew,
  Event,
  HumanQuestionPayload,
  Mission,
  SessionUpdatedEvent,
  Subject,
  WarningEvent,
} from "../lib/types";
import { DuplicateSubjectOverlay } from "../components/DuplicateSubjectOverlay";
import {
  isSecondaryFor,
  useCurrentWindowLabel,
  useReportSubject,
  useWindowFocus,
} from "../lib/windowFocus";
import { EventFeed } from "../components/EventFeed";
import { MissionInput } from "../components/MissionInput";
import { MissionMetaPanel } from "../components/MissionMetaPanel";
import { MissionResetConfirm } from "../components/MissionResetConfirm";
import { RunnersRail } from "../components/RunnersRail";
import {
  RunnerTerminal,
  type RunnerTerminalHandle,
} from "../components/RunnerTerminal";
import {
  ArchivingOverlay,
  ResumingOverlay,
  SessionEndedOverlay,
  StartingOverlay,
} from "../components/SessionEndedOverlay";
import {
  ResumeButton,
  StopButton,
} from "../components/ui/SessionControl";
import { chunkIndicatesTuiReady, isFreshSpawn } from "../lib/sessionLifecycle";
import { terminalGridFromElement } from "../lib/terminalSizing";
import { useDelayedFlag } from "../lib/useDelayedFlag";
import { useResizableWidth } from "../hooks/useResizableWidth";
import { useTerminalBg } from "../lib/useTerminalBg";
import {
  clearLastMissionTerminalId,
  getLastMissionTerminalId,
  setLastMissionTerminalId,
} from "../lib/missionLastTerminal";
import {
  markArchivingMission,
  unmarkArchivingMission,
  useArchivingMission,
} from "../lib/archivingState";

const RAIL_STORAGE_WIDTH = "runner.mission.rail.width";
const RAIL_MIN = 200;
const RAIL_MAX = 480;
const RAIL_DEFAULT = 288;

/// Compute cols/rows for the would-be terminal area from the Pane
/// container's bounding rect + xterm/FitAddon using the user's current
/// terminal font settings. Returns null if the container has no rect
/// (workspace not mounted yet) or xterm cannot measure.
///
/// Used by `resumeMission`'s fallback chain when no individual slot
/// terminal is measurable — e.g. the user clicked Resume from the
/// feed tab and no slot was ever activated, so every slot pane is
/// still `display:none` and every `RunnerTerminal.measure()` returns
/// null. The cell size we compute here is close to but not exactly
/// xterm's CharSizeService output; a small drift is acceptable because
/// the agent paints at whatever cols we pass and xterm soft-wraps from
/// there if needed. "Approximately correct" beats "spawn at 80×24."
function workspaceDimsFromContainer(
  container: HTMLElement,
): { cols: number; rows: number } | null {
  return terminalGridFromElement(container);
}

export default function MissionWorkspace() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const t = useT();
  const [mission, setMission] = useState<Mission | null>(null);
  const [crew, setCrew] = useState<Crew | null>(null);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [events, setEvents] = useState<Event[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Resume-fallback banner; non-blocking advisory (see types.ts WarningEvent).
  const [warning, setWarning] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"feed" | string>("feed"); // string = sessionId
  // Session ids whose PTY tabs are currently visible in the strip.
  // Feed is always shown and not in this set. PTY tabs open on demand
  // when the user clicks a runner card in the right rail; closing a
  // tab removes it here and (if it was active) snaps activeTab back
  // to feed. Pane mounts are filtered by this set so closing a tab
  // also unmounts xterm — matches the "tab is gone" mental model.
  const [openTabs, setOpenTabs] = useState<string[]>([]);
  const seenIdsRef = useRef<Set<string>>(new Set());
  // Live handles to each open slot's xterm, keyed by session id. The
  // resume path measures the actual cols/rows before calling the
  // backend so the new PTY is forked at the right size instead of
  // pty_runtime's 80×24 fallback (#resume-pty-size-mismatch).
  const terminalsRef = useRef<Map<string, RunnerTerminalHandle>>(new Map());
  // Wrapping element around every Pane (feed + per-slot PTY panes).
  // Used by `resumeMission` as a last-resort source for cols/rows when
  // no individual terminal is measurable — every Pane is
  // `absolute inset-0` inside this container, so its rect is the size
  // any pane *would* have if activated. See `resumeMission` for the
  // fallback chain and `workspaceDimsFromContainer` for the cell-size
  // measurement step.
  const paneContainerRef = useRef<HTMLDivElement | null>(null);

  // Multi-window coordination (impl 0018). This window reports the mission as
  // its subject; if another window holds the same mission with a later focus,
  // we're the secondary and must not own the PTY — no terminal mount, no
  // stdin/resize/start. The mission/session metadata still loads so the feed
  // and overlay can render.
  const focusMap = useWindowFocus();
  const myWindowLabel = useCurrentWindowLabel();
  const subject = useMemo<Subject | null>(
    () => (id ? { type: "Mission", value: id } : null),
    [id],
  );
  useReportSubject(subject);
  const { secondary: isSecondary, primaryLabel } = isSecondaryFor(
    focusMap,
    myWindowLabel,
    subject,
  );
  const [overlayDismissed, setOverlayDismissed] = useState(false);
  // Re-show the overlay when the subject changes or this window (re)gains
  // secondary status — e.g. another window steals focus mid-session.
  useEffect(() => {
    setOverlayDismissed(false);
  }, [id, isSecondary]);
  const showDuplicateOverlay = isSecondary && !overlayDismissed;
  // Force the feed view while secondary: the PTY tabs/panes don't render, so
  // a non-feed activeTab would otherwise leave a blank content area.
  const feedActive = isSecondary || activeTab === "feed";

  // Combined: subscribe to `event/appended` BEFORE running the
  // events_replay query, then merge both into a single ULID-deduped
  // event list. Without this ordering, an event the worker appends
  // between the replay query returning and the listener attaching falls
  // through to the floor — the bus already delivered it, but no one was
  // listening yet. The bus's initial replay also hands back historical
  // events via the same listener, so we dedup on both sides.
  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    seenIdsRef.current = new Set();
    setMission(null);
    setCrew(null);
    setSessions([]);
    setEvents([]);
    setActiveTab("feed");
    setOpenTabs([]);
    setError(null);
    setLoading(true);

    const ingest = (newEvents: Event[]) => {
      if (newEvents.length === 0) return;
      const seen = seenIdsRef.current;
      const fresh = newEvents.filter((e) => !seen.has(e.id));
      if (fresh.length === 0) return;
      for (const e of fresh) seen.add(e.id);
      // Preserve append order: events from the replay are already
      // sorted by ULID; events from the bus arrive in append order.
      // Re-sort the merged tail so we never display out-of-order rows
      // (a late-arriving replay event whose id sorts before a
      // bus-delivered one would otherwise show up at the bottom).
      setEvents((prev) => {
        const merged = [...prev, ...fresh];
        merged.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
        return merged;
      });
    };

    void (async () => {
      try {
        const fn = await listen<AppendedEvent>("event/appended", (msg) => {
          const ev = msg.payload;
          if (ev.mission_id !== id) return;
          ingest([ev.event]);
        });
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;

        // Attach FIRST: rebuilds router/bus if we just relaunched the
        // app. Idempotent — a no-op for missions whose router is
        // already mounted (e.g. just navigating in & out of the
        // workspace). Without this, the event/appended listener above
        // would tail an empty bus and resumed slot PTYs would never
        // see stdin pushes.
        await api.mission.attach(id);
        const [m, ss, evs] = await Promise.all([
          api.mission.get(id),
          api.session.list(id),
          api.mission.eventsReplay(id),
        ]);
        if (cancelled) return;
        setMission(m);
        setSessions(ss);
        // Crew is needed by the right rail's mission-meta view for
        // the crew name link + goal fallback. Best-effort: a failure
        // here shouldn't block the workspace from rendering.
        api.crew
          .get(m.crew_id)
          .then((c) => {
            if (!cancelled) setCrew(c);
          })
          .catch((e) => console.error("MissionWorkspace: crew_get failed", e));
        const rememberedSessionId = getLastMissionTerminalId(id);
        const rememberedSession =
          m.archived_at == null && rememberedSessionId
            ? ss.find(
                (s) => s.id === rememberedSessionId && s.mission_id === id,
              )
            : undefined;
        if (m.archived_at != null) {
          clearLastMissionTerminalId(id);
          setOpenTabs([]);
          setActiveTab("feed");
        } else {
          // Auto-open every slot's PTY tab on mount. The user can close
          // individual tabs via the × on each tab; if they close them
          // all and re-mount, the mount path opens them again. Keep
          // the remembered tab in the strip before selecting it so a
          // future change to lazy-open tabs can't select a hidden pane.
          const nextOpenTabs = ss.map((s) => s.id);
          if (
            rememberedSession &&
            !nextOpenTabs.includes(rememberedSession.id)
          ) {
            nextOpenTabs.push(rememberedSession.id);
          }
          setOpenTabs(nextOpenTabs);
          if (rememberedSession) {
            setActiveTab(rememberedSession.id);
          } else {
            if (rememberedSessionId) clearLastMissionTerminalId(id);
            setActiveTab("feed");
          }
        }
        ingest(evs);
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
      seenIdsRef.current = new Set();
    };
  }, [id]);

  // MCP mutations (pin, rename, archive, reset) don't have a direct
  // invoke response in this webview. Refresh the mission row when the
  // backend announces that mission metadata changed.
  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen("mission/changed", () => {
      void Promise.all([api.mission.get(id), api.session.list(id)])
        .then(([m, ss]) => {
          if (cancelled) return;
          setMission(m);
          setSessions(ss);
        })
        .catch(() => {
          // best-effort; the next workspace reload or bus event will retry
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
  }, [id]);

  // Surface non-fatal session warnings (today: agent-resume fallback).
  // Filter on mission_id so warnings from other workspaces don't leak.
  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<WarningEvent>("session/warning", (event) => {
      if (event.payload.mission_id !== id) return;
      setWarning(event.payload.message);
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
  }, [id]);

  // Refresh session statuses when a session/exit event lands. Without
  // this, the rail's status dots stay green even after a runner crashes
  // — the bus's `event/appended` doesn't tell us about PTY-level state,
  // only about the audit-log envelopes.
  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<{ mission_id: string | null }>("session/exit", (event) => {
      if (event.payload.mission_id !== id) return;
      void api.session
        .list(id)
        .then((rows) => {
          if (cancelled) return;
          setSessions(rows);
        })
        .catch(() => {
          // best-effort — surface only persistent failures via the
          // initial-load error path
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
  }, [id]);

  useEffect(() => {
    if (!id) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<SessionUpdatedEvent>("session/updated", (event) => {
      if (event.payload.mission_id !== id) return;
      void api.session
        .list(id)
        .then((rows) => {
          if (cancelled) return;
          setSessions(rows);
        })
        .catch(() => {
          // best-effort — the next lifecycle refresh or reload will
          // reconcile metadata if this request fails
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
  }, [id]);

  const pendingCodexKeySessionIds = useMemo(
    () =>
      sessions
        .filter(
          (s) =>
            s.status === "running" &&
            s.runtime === "codex" &&
            !s.agent_session_key,
        )
        .map((s) => s.id)
        .sort()
        .join("|"),
    [sessions],
  );

  useEffect(() => {
    if (!id || !pendingCodexKeySessionIds || mission?.status !== "running") {
      return;
    }

    const MAX_ATTEMPTS = 35;
    let cancelled = false;
    let attempts = 0;

    const poll = () => {
      attempts += 1;
      void api.session
        .list(id)
        .then((rows) => {
          if (cancelled) return;
          setSessions(rows);
          const stillPending = rows.some(
            (s) =>
              s.status === "running" &&
              s.runtime === "codex" &&
              !s.agent_session_key,
          );
          if (!stillPending || attempts >= MAX_ATTEMPTS) {
            window.clearInterval(interval);
          }
        })
        .catch(() => {
          if (attempts >= MAX_ATTEMPTS) window.clearInterval(interval);
        });
    };

    const interval = window.setInterval(poll, 1000);
    poll();
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [id, mission?.status, pendingCodexKeySessionIds]);

  // Effective mission goal — read from the `mission_goal` event the
  // backend writes at mission_start. This is the only authoritative
  // source: the crew's `goal` column drifts if a user edits the crew
  // default after launching, and `mission.goal_override` is set only
  // when an override was passed at start. Returns `null` while events
  // are still loading so the rail can show a "Loading…" placeholder.
  const missionGoal = useMemo<string | null>(() => {
    if (events.length === 0) return null;
    for (const e of events) {
      if (e.kind === "signal" && e.type === "mission_goal") {
        const text = (e.payload as { text?: unknown }).text;
        return typeof text === "string" ? text : "";
      }
    }
    return "";
  }, [events]);

  // Lead handle resolved from the crew_runners.lead flag returned by
  // session_list. Fall back to roster order only for malformed/old rows.
  const leadHandle = useMemo(() => {
    if (sessions.length === 0) return "";
    return sessions.find((s) => s.lead)?.handle ?? sessions[0].handle;
  }, [sessions]);

  // Stop = kill all live PTYs in the mission. Mission row stays
  // `running`; per-slot Resume buttons reanimate them. Cheap, reversible.
  const stopMission = useCallback(async () => {
    if (!mission) return;
    try {
      const next = await api.mission.stop(mission.id);
      setMission(next);
      const rows = await api.session.list(mission.id);
      setSessions(rows);
    } catch (e) {
      setError(String(e));
    }
  }, [mission]);

  const pinMission = useCallback(async () => {
    if (!mission) return;
    try {
      const next = await api.mission.pin(mission.id, !mission.pinned_at);
      setMission(next);
    } catch (e) {
      setError(String(e));
    }
  }, [mission]);

  // Topbar rename uses a `prompt()` rather than an inline input —
  // keeps the topbar layout fixed and avoids fiddly focus management
  // around a button-edge input. The sidebar still has the inline
  // rename row for power users.
  const renameMissionPrompt = useCallback(async () => {
    if (!mission) return;
    const next = window.prompt(t("Rename mission"), mission.title);
    if (next === null) return; // user cancelled
    const trimmed = next.trim();
    if (!trimmed || trimmed === mission.title) return;
    try {
      const updated = await api.mission.rename(mission.id, trimmed);
      setMission(updated);
    } catch (e) {
      setError(String(e));
    }
  }, [mission]);

  // Archive = end of mission. Status flips to `completed`, router/bus
  // unmount, no further sessions can spawn against this mission.
  // Mirrors `Sidebar.archiveMission`: no confirm dialog (the kebab
  // affordance is intentional + the action surfaces the danger
  // styling), bounce off the workspace once archived (the page would
  // refuse to attach a completed mission's router and look broken),
  // and defer the unmark past the navigate commit so the still-
  // mounted workspace doesn't briefly re-render with archivingMission
  // false while React 18 batches the sync emit with the route change.
  const archiveMission = useCallback(async () => {
    if (!mission) return;
    markArchivingMission(mission.id);
    try {
      clearLastMissionTerminalId(mission.id);
      await api.mission.archive(mission.id);
      navigate("/runners");
    } catch (e) {
      setError(String(e));
    } finally {
      setTimeout(() => unmarkArchivingMission(mission.id), 0);
    }
  }, [mission, navigate]);

  // Reset = wipe the run, respawn slots, keep the mission row. Used
  // for testing — you get the same mission back with a clean event
  // log and fresh PTYs. Confirmed via a modal (`MissionResetConfirm`)
  // because event-log loss is hard to undo.
  const [resetConfirmOpen, setResetConfirmOpen] = useState(false);
  const resetMission = useCallback(async () => {
    if (!mission) return;
    try {
      const next = await api.mission.reset(mission.id);
      setMission(next);
      // Refresh sessions + events. The reset path archives the old
      // session rows and inserts fresh ones, so session_list returns
      // the new set of running slots. The event log was wiped + has
      // only the two opening events; eventsReplay picks them up.
      const [rows, evs] = await Promise.all([
        api.session.list(mission.id),
        api.mission.eventsReplay(mission.id),
      ]);
      setSessions(rows);
      // Clear stale events + ingest fresh — bypassing the seenIds
      // dedup since the new events have new ULIDs we haven't seen.
      seenIdsRef.current = new Set();
      setEvents([]);
      const fresh = evs.filter((e) => {
        if (seenIdsRef.current.has(e.id)) return false;
        seenIdsRef.current.add(e.id);
        return true;
      });
      setEvents(fresh);
      // Fresh slots = open all PTY tabs.
      clearLastMissionTerminalId(mission.id);
      setOpenTabs(rows.map((s) => s.id));
      setActiveTab("feed");
      setResetConfirmOpen(false);
    } catch (e) {
      setError(String(e));
    }
  }, [mission, setResetConfirmOpen]);

  // Resume all = iterate stopped/crashed sessions and respawn each.
  // Hits the same `session_resume` path the per-slot Resume button
  // uses; just saves clicks when every slot needs to come back.
  const [resumingAll, setResumingAll] = useState(false);
  const resumeMission = useCallback(async () => {
    if (!mission) return;
    setResumingAll(true);
    let firstErr: string | null = null;
    try {
      // Pre-walk for a shared fallback dim. Hidden tabs (display:none
      // Pane wrappers) have 0×0 rects so their `measure()` may return
      // null; without a fallback the resume RPC sends (null, null),
      // backend spawns at 80×24, and the agent paints its `--resume`
      // conversation history at 80 cols. For main-screen TUIs those
      // hard-wrapped narrow lines stick in scrollback. Three-tier
      // fallback:
      //   1. The clicked-from tab's own `measure()` (always works when
      //      a slot tab is active).
      //   2. Any other slot terminal that has been activated before
      //      and remembers its last-fit dims (every Pane shares the
      //      same container rect, so any one's dims work for all).
      //   3. A direct measurement of the Pane container + DOM cell-
      //      size probe. This catches the "Resume clicked from the
      //      feed tab with no slot ever activated" path — every slot
      //      terminal is still at the 80×24 sentinel so (1)/(2) both
      //      return null.
      let sharedDims: { cols: number; rows: number } | null = null;
      for (const s of sessions) {
        const d = terminalsRef.current.get(s.id)?.measure();
        if (d) {
          sharedDims = d;
          break;
        }
      }
      if (!sharedDims && paneContainerRef.current) {
        sharedDims = workspaceDimsFromContainer(paneContainerRef.current);
      }
      // Best-effort over every stopped slot. Don't bail on the first
      // failure — earlier slots may have already resumed, and the
      // user wants the UI to reflect whatever actually came up.
      // Errors are collected and surfaced after the refresh.
      for (const s of sessions) {
        if (s.status === "running") continue;
        const dims = terminalsRef.current.get(s.id)?.measure() ?? sharedDims;
        try {
          await api.session.resume(
            s.id,
            dims?.cols ?? null,
            dims?.rows ?? null,
          );
        } catch (e) {
          if (firstErr == null) firstErr = String(e);
        }
      }
    } finally {
      // Refresh in finally so a partial failure (one slot resumed,
      // a later one threw) still updates the row list + opens tabs
      // for the slots that did come back. Without this the UI stays
      // stuck reading "paused" while the resumed PTYs are live.
      try {
        const rows = await api.session.list(mission.id);
        setSessions(rows);
        // Mission Resume implies the user wants to see the slots
        // come back to life. Reopen any tabs they'd previously
        // closed — resume isn't a useful action if the panes are
        // hidden.
        setOpenTabs((prev) => {
          const next = new Set(prev);
          for (const r of rows) next.add(r.id);
          return Array.from(next);
        });
      } catch (e) {
        if (firstErr == null) firstErr = String(e);
      }
      if (firstErr != null) setError(firstErr);
      setResumingAll(false);
    }
  }, [mission, sessions]);

  const archivingMission = useArchivingMission(mission?.id);

  const allSessionsLive =
    sessions.length > 0 && sessions.every((s) => s.status === "running");
  const anySessionStopped =
    sessions.length > 0 && sessions.some((s) => s.status !== "running");
  // Coordination still works as long as ≥1 slot is alive — the human
  // can talk to whichever runner is up. Only block input + show the
  // pause overlay when literally zero PTYs are running. A mid-mission
  // crash on one worker shouldn't gate human-to-lead messaging.
  const anySessionLive =
    sessions.length > 0 && sessions.some((s) => s.status === "running");
  // archived_at is the single discriminator across the workspace —
  // status pill, no-PTY render branch, hidden actions. We don't key
  // any UX off `status === 'completed'` because the migration may
  // later widen archive to include other terminal states.
  const isArchived = mission?.archived_at != null;

  useEffect(() => {
    if (!id || loading || !mission || mission.id !== id) return;
    if (isArchived) {
      clearLastMissionTerminalId(id);
      setActiveTab("feed");
      setOpenTabs((prev) => (prev.length === 0 ? prev : []));
      return;
    }

    const validSessionIds = new Set(
      sessions
        .filter((s) => s.mission_id === id)
        .map((s) => s.id),
    );
    const rememberedSessionId = getLastMissionTerminalId(id);
    if (rememberedSessionId && !validSessionIds.has(rememberedSessionId)) {
      clearLastMissionTerminalId(id);
    }
    if (activeTab !== "feed" && !validSessionIds.has(activeTab)) {
      setActiveTab("feed");
    }
    setOpenTabs((prev) => {
      const next = prev.filter((tabId) => validSessionIds.has(tabId));
      return next.length === prev.length ? prev : next;
    });
  }, [activeTab, id, isArchived, loading, mission, sessions]);

  const selectFeed = useCallback(() => {
    setActiveTab("feed");
  }, []);

  const selectPty = useCallback(
    (sessionId: string) => {
      if (!id || isArchived) return;
      const session = sessions.find(
        (s) => s.id === sessionId && s.mission_id === id,
      );
      if (!session) {
        const rememberedSessionId = getLastMissionTerminalId(id);
        if (rememberedSessionId === sessionId) {
          clearLastMissionTerminalId(id);
        }
        setActiveTab("feed");
        return;
      }
      setOpenTabs((prev) =>
        prev.includes(sessionId) ? prev : [...prev, sessionId],
      );
      setLastMissionTerminalId(id, sessionId);
      setActiveTab(sessionId);
    },
    [id, isArchived, sessions],
  );

  const shortcutTabs = useMemo<Array<"feed" | string>>(() => {
    if (isArchived) return ["feed"];
    const slotTabs = openTabs
      .map((tabId) => sessions.find((s) => s.id === tabId))
      .filter((s): s is SessionRow => s !== undefined)
      .map((s) => s.id);
    return ["feed", ...slotTabs].slice(0, 9);
  }, [isArchived, openTabs, sessions]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!isModKey(e) || isOppositeMod(e) || e.altKey || e.shiftKey) return;
      if (!/^[1-9]$/.test(e.key)) return;
      const target = shortcutTabs[Number(e.key) - 1];
      if (!target) return;
      e.preventDefault();
      e.stopPropagation();
      if (target === "feed") selectFeed();
      else selectPty(target);
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, [selectFeed, selectPty, shortcutTabs]);

  // Project ask_human → human_question pairings + human_response
  // resolutions out of the feed. Mirrors the router's reconstruct_from_log
  // logic so the UI can render the right state on reopen even before the
  // bus has redelivered anything.
  const { askersByQuestion, resolvedAsks } = useMemo(() => {
    const askHumanAskers: Record<string, string> = {};
    const askersByQuestion: Record<string, string> = {};
    const resolvedAsks: Record<string, string> = {};
    for (const ev of events) {
      if (ev.kind !== "signal" || !ev.type) continue;
      if (ev.type === "ask_human") {
        askHumanAskers[ev.id] = ev.from;
      } else if (ev.type === "human_question") {
        const p = ev.payload as HumanQuestionPayload | null;
        const askId = p?.triggered_by;
        if (askId && askHumanAskers[askId]) {
          askersByQuestion[ev.id] = askHumanAskers[askId];
          delete askHumanAskers[askId];
        }
      } else if (ev.type === "human_response") {
        const p = ev.payload as { question_id?: string; choice?: string } | null;
        if (p?.question_id) {
          resolvedAsks[p.question_id] = p.choice ?? "";
        }
      }
    }
    return { askersByQuestion, resolvedAsks };
  }, [events]);

  // Latest runner_status (busy/idle) per handle for the rail badge.
  const runnerStatusMap = useMemo(() => {
    const map: Record<string, "busy" | "idle"> = {};
    for (const ev of events) {
      if (
        ev.kind === "signal" &&
        ev.type === "runner_status" &&
        ev.from
      ) {
        const state = (ev.payload as { state?: string } | null)?.state;
        if (state === "busy" || state === "idle") map[ev.from] = state;
      }
    }
    return map;
  }, [events]);

  const onOpenPty = useCallback(
    (sessionId: string) => {
      selectPty(sessionId);
    },
    [selectPty],
  );

  const onCloseTab = useCallback(
    (sessionId: string) => {
      setOpenTabs((prev) => prev.filter((id) => id !== sessionId));
      if (id && getLastMissionTerminalId(id) === sessionId) {
        clearLastMissionTerminalId(id);
      }
      setActiveTab((prev) => (prev === sessionId ? "feed" : prev));
    },
    [id],
  );

  const handles = sessions.map((s) => s.handle);
  const startedAt = mission ? formatRelativeTime(mission.started_at, t) : "";
  const [kebabOpen, setKebabOpen] = useState(false);
  // Right rail (Runners panel) collapse state. Mirrors the RunnerChat
  // side-panel collapse — same localStorage shape for consistency.
  const [railOpen, setRailOpen] = useState<boolean>(() => {
    if (typeof localStorage === "undefined") return true;
    return localStorage.getItem("runner.mission.rail.open") !== "0";
  });
  useEffect(() => {
    try {
      localStorage.setItem("runner.mission.rail.open", railOpen ? "1" : "0");
    } catch {
      // ignore storage errors
    }
  }, [railOpen]);
  // Drag-to-resize width for the rail. Persisted separately from the
  // open/closed flag so collapse → expand restores the last dragged
  // width instead of the hardcoded default. The hook writes
  // style.width directly through the refs during drag — avoids the
  // RunnersRail / MissionMetaPanel subtree re-rendering per frame.
  const railAsideRef = useRef<HTMLElement>(null);
  const railInnerRef = useRef<HTMLDivElement>(null);
  const { width: railWidth, onResizeStart: onRailResizeStart } =
    useResizableWidth({
      storageKey: RAIL_STORAGE_WIDTH,
      defaultWidth: RAIL_DEFAULT,
      min: RAIL_MIN,
      max: RAIL_MAX,
      edge: "left",
      targets: [railAsideRef, railInnerRef],
    });
  // Right rail content toggle — Runners roster vs Mission meta. Persists
  // per-app (not per-mission): the user's preference for which view to
  // see on entry is consistent across missions.
  const [railView, setRailView] = useState<"runners" | "meta">(() => {
    if (typeof localStorage === "undefined") return "runners";
    return localStorage.getItem("runner.mission.rail.view") === "meta"
      ? "meta"
      : "runners";
  });
  useEffect(() => {
    try {
      localStorage.setItem("runner.mission.rail.view", railView);
    } catch {
      // ignore storage errors
    }
  }, [railView]);

  // Keep a render gate while mission/session/event data loads: the
  // tabs/feed/rail below assume `mission` is non-null. Show only a
  // neutral delayed loading pill here; route switching is not a start
  // or resume action.
  const isLoading = loading || !mission;
  const showLoadingPill = useDelayedFlag(isLoading, 150, id);

  return (
    // flex-row outer so the right rail becomes a top-level sibling
    // of the main column. The rail then spans the full workspace
    // height, with its own header that lines up with the topbar
    // across the divider — same layout shape as RunnerChat.
    <div className="flex h-full flex-1 flex-row bg-bg">
      <div className="flex min-w-0 flex-1 flex-col">
      {/* `data-tauri-drag-region` makes the entire header strip drag
          the window, matching macOS toolbar behavior. Buttons inside
          (Resume / Stop / kebab / panel toggle) keep their click
          handlers — Tauri only enters drag mode on mousedowns that
          land on the bare header, not on interactive children. */}
      <header
        data-tauri-drag-region
        className="flex items-center justify-between gap-4 border-b border-line bg-panel px-6 pb-3.5 pt-9"
      >
        <div className="flex min-w-0 items-center gap-3.5">
          {/* Mission glyph — matches Pencil node `nEpyL`: a 36×36
              rounded square with a lucide `flag` icon at 18px in the
              accent green. */}
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-line bg-bg text-accent">
            <Flag aria-hidden className="h-[18px] w-[18px]" />
          </div>
          <div className="flex min-w-0 flex-col gap-0.5">
            <div className="flex items-center gap-2">
              <h1 className="truncate text-[14px] font-semibold leading-tight text-fg">
                {mission?.title ?? "…"}
              </h1>
              {/* Type badge mirrors RunnerChat's "Chat" pill so the
                  workspace surfaces consistently signal what kind of
                  thing this is, not just its status. */}
              <span className="rounded bg-line-strong px-2 py-px text-[9px] font-bold uppercase tracking-[0.5px] text-fg-2">
                {t("Mission")}
              </span>
              {/* Status pill moved to the left so it sits next to
                  the title instead of competing visually with the
                  Resume / Stop / kebab cluster on the right — the
                  action buttons already imply the current state,
                  and a pill at the same edge read redundant. */}
              {mission ? (() => {
                type Display =
                  | "running"
                  | "stopped"
                  | "archived"
                  | "aborted"
                  | "resuming";
                // archived_at short-circuits at the top. The
                // remaining branches only see non-archived rows, so
                // any status='completed' here would mean a row that
                // missed the migration backfill — fall through to
                // 'aborted' since it's terminal-but-not-archived and
                // the worker pill copy reads correctly for triage.
                const display: Display = isArchived
                  ? "archived"
                  : resumingAll
                    ? "resuming"
                    : mission.status === "running"
                      ? anySessionLive
                        ? "running"
                        : "stopped"
                      : "aborted";
                const pillClass =
                  display === "running"
                    ? "bg-accent/15 text-accent"
                    : display === "aborted"
                      ? "bg-danger/15 text-danger"
                      : display === "resuming"
                        ? "bg-info/15 text-info"
                        : "bg-raised text-fg-2";
                const dotClass =
                  display === "running"
                    ? "bg-accent"
                    : display === "aborted"
                      ? "bg-danger"
                      : display === "resuming"
                        ? "bg-info"
                        : "bg-fg-3";
                return (
                  <span
                    className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${pillClass}`}
                  >
                    <span
                      className={`inline-flex h-1.5 w-1.5 rounded-full ${dotClass}`}
                    />
                    {display === "resuming" ? t("resuming…") : t(display)}
                  </span>
                );
              })() : null}
              {/* Unambiguous read-only affordance for archived
                  missions — the status pill alone reads too easily
                  as just another state. Muted chip beside the title
                  so the workspace clearly communicates that nothing
                  here will accept input. */}
              {isArchived ? (
                <span className="inline-flex shrink-0 items-center rounded border border-line bg-raised px-2 py-0.5 text-[10px] font-medium text-fg-2">
                  {t("Archived · read-only")}
                </span>
              ) : null}
            </div>
            <span className="truncate text-[11px] leading-tight text-fg-3">
              {sessions.length === 1
                ? t("{n} runner", { n: sessions.length })
                : t("{n} runners", { n: sessions.length })}
              {startedAt ? t(" · started {time}", { time: startedAt }) : ""}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {/* Secondary windows (impl 0018) are read-only: Resume/Reset
              respawn PTYs, Stop kills them, Archive ends the mission — all
              of which act on PTYs the primary window owns. Hide the whole
              action cluster while secondary; focus the primary to act. */}
          {mission?.status === "running" && !resumingAll && !isSecondary ? (
            <>
              {anySessionStopped ? (
                <ResumeButton
                  onClick={() => void resumeMission()}
                  title={t("Respawn every stopped slot in this mission")}
                />
              ) : null}
              {allSessionsLive ? (
                <StopButton
                  onClick={() => void stopMission()}
                  title={t("Kill all PTYs; mission stays running so you can Resume")}
                  iconTone="fg"
                />
              ) : null}
              <MissionKebab
                pinned={!!mission.pinned_at}
                open={kebabOpen}
                onToggle={() => setKebabOpen((v) => !v)}
                onClose={() => setKebabOpen(false)}
                onPin={() => {
                  setKebabOpen(false);
                  void pinMission();
                }}
                onRename={() => {
                  setKebabOpen(false);
                  void renameMissionPrompt();
                }}
                onReset={() => {
                  setKebabOpen(false);
                  setResetConfirmOpen(true);
                }}
                onArchive={() => {
                  setKebabOpen(false);
                  void archiveMission();
                }}
              />
            </>
          ) : null}
          {!railOpen ? (
            <button
              type="button"
              onClick={() => setRailOpen(true)}
              title={t("Open runners panel")}
              aria-label={t("Open runners panel")}
              className="inline-flex h-7 w-7 items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg"
            >
              <PanelRightDashed aria-hidden className="h-4 w-4" />
            </button>
          ) : null}
        </div>
      </header>

      {error ? (
        <div className="mx-8 mt-3 rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
          {error}
        </div>
      ) : null}

      {warning ? (
        <div className="mx-8 mt-3 flex items-start justify-between gap-3 rounded border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn">
          <span>{warning}</span>
          <button
            type="button"
            onClick={() => setWarning(null)}
            className="cursor-pointer text-xs text-warn/80 hover:text-warn"
          >
            {t("Dismiss")}
          </button>
        </div>
      ) : null}

      {isLoading ? (
        showLoadingPill ? (
          <StartingOverlay label={t("Loading mission…")} inline />
        ) : (
          <div className="flex flex-1 min-h-0" />
        )
      ) : (
        <div className="flex flex-1 min-h-0 flex-col">
          <div className="flex h-[38px] items-end gap-1 border-b border-line bg-panel px-6">
            <TabButton
              active={feedActive}
              onClick={selectFeed}
              shortcut={`${MOD_LABEL}1`}
            >
              {t("feed")}
            </TabButton>
            {/* Archived missions render feed only — skip the per-PTY
                tabs so no xterm canvas ever mounts. Secondary windows
                (impl 0018) likewise show feed only: the duplicated
                terminal lives in the primary window. The Pane block
                below applies the same gate. */}
            {!isArchived && !isSecondary
              ? openTabs
                  .map((tabId) => sessions.find((s) => s.id === tabId))
                  .filter((s): s is SessionRow => s !== undefined)
                  .map((s, index) => (
                    <PtyTabButton
                      key={s.id}
                      handle={s.handle}
                      active={activeTab === s.id}
                      onClick={() => selectPty(s.id)}
                      onClose={() => onCloseTab(s.id)}
                      shortcut={index < 8 ? `${MOD_LABEL}${index + 2}` : undefined}
                    />
                  ))
              : null}
          </div>

          <div
            ref={paneContainerRef}
            className="relative flex flex-1 min-h-0 flex-col"
          >
            {/* All panes stay mounted so xterm's in-memory scrollback
                survives tab switches. Inactive panes use display:none:
                that keeps the React/xterm instances alive while making
                the visible session unambiguous. The terminal activation
                effect refits + replays after the pane is shown. */}
            <Pane active={feedActive}>
              <EventFeed
                missionId={mission.id}
                events={events}
                resolvedAsks={resolvedAsks}
                askersByQuestion={askersByQuestion}
                active={feedActive}
                onError={setError}
              />
              {/* Secondary windows can't send input: human_said is
                  injected into the lead's PTY stdin by the router, which
                  the primary owns (impl 0018). Disable while secondary. */}
              <MissionInput
                missionId={mission.id}
                leadHandle={leadHandle}
                handles={handles}
                disabled={
                  mission.status !== "running" || !allSessionsLive || isSecondary
                }
                onError={setError}
              />
              {/* Pause overlay fires whenever *any* slot is stopped.
                  Per the "no single-slot resume" rule, a partial-
                  mission state (one worker crashed, lead still up)
                  isn't a valid run. Full-pane backdrop + centered
                  card so the feed reads as paused at a glance —
                  the inline variant the slot panes use sits over
                  the input and was easy to miss when the feed
                  scrolled. */}
              {mission.status === "running" &&
              !allSessionsLive &&
              !resumingAll &&
              !archivingMission &&
              !isSecondary ? (
                // Scrim is rendered by the overlay itself (issue #173).
                // Hidden while secondary (impl 0018) so its Resume/Archive
                // buttons can't act on PTYs the primary owns.
                <MissionPausedCard
                  anySessionLive={anySessionLive}
                  onResumeMission={() => void resumeMission()}
                  onArchiveMission={() => void archiveMission()}
                />
              ) : null}
            </Pane>

            {/* Skip per-session PTY panes for archived missions and for
                secondary windows (impl 0018) so no xterm canvas ever
                mounts and we never write to a PTY the primary owns. On
                flip primary→secondary this unmounts the terminals (and
                clears their refs via the register callback); on flip
                back, they remount and re-attach. The feed Pane stays
                rendered above as the only surface. */}
            {!isArchived && !isSecondary
              ? openTabs
                  .map((tabId) => sessions.find((s) => s.id === tabId))
                  .filter((s): s is SessionRow => s !== undefined)
                  .map((s) => (
                    <Pane key={s.id} active={activeTab === s.id}>
                      <SlotPtyPane
                        session={s}
                        active={activeTab === s.id}
                        forcedResuming={resumingAll && !archivingMission}
                        anySessionLive={anySessionLive}
                        onError={setError}
                        onResumeMission={() => void resumeMission()}
                        onArchiveMission={() => void archiveMission()}
                        registerTerminal={(handle) => {
                          if (handle) terminalsRef.current.set(s.id, handle);
                          else terminalsRef.current.delete(s.id);
                        }}
                      />
                    </Pane>
                  ))
              : null}
            {/* Centered amber pill + scrim while a mission archive
                is in flight — fired from either the sidebar kebab
                or this workspace's own kebab. Scrim matches the
                RunnerChat archive flow so the destructive
                transition is unambiguous; without it the pill
                flashes briefly over a still-live-looking feed and
                is easy to miss. Strictly mutually exclusive with
                the resuming-all overlay (slot panes gate
                forcedResuming on !archivingMission). */}
            {archivingMission ? <ArchivingOverlay withScrim /> : null}
            {/* Arc-style duplicate-subject overlay (impl 0018). Sits
                above the panes; gates nothing itself (the PTY mount is
                gated by `isSecondary`) — "Stay here" only hides the
                card so the user can read the feed underneath. */}
            {showDuplicateOverlay ? (
              <DuplicateSubjectOverlay
                kind="mission"
                primaryLabel={primaryLabel}
                onStayHere={() => setOverlayDismissed(true)}
              />
            ) : null}
          </div>
        </div>
      )}
      </div>
      {/* Collapsible right rail — top-level sibling of the main
          column so it spans the full workspace height (header
          included). Mirrors the RunnerChat pattern: the inner
          wrapper stays mounted at the persisted width and clipped
          by overflow-hidden so width animates without reflowing
          the children. A 4px drag strip on the left edge resizes
          the rail. */}
      <aside
        ref={railAsideRef}
        aria-hidden={!railOpen}
        style={{ width: railOpen ? railWidth : 0 }}
        className={`relative flex shrink-0 flex-col overflow-hidden bg-panel transition-[width,border-left-width] duration-200 ease-in-out ${
          railOpen ? "border-l border-line" : "border-l-0"
        }`}
      >
        <div
          ref={railInnerRef}
          style={{ width: railWidth }}
          className="flex h-full flex-col"
        >
          {/* Rail header — same px-5 / pt-9 / pb-3.5 / border-b
              rhythm as the workspace topbar so the divider lines up
              across the column boundary. The icon strip on the left
              flips between Runners (roster) and Mission (meta); the
              collapse button stays on the right.
              `data-tauri-drag-region` keeps the rail header in the
              same draggable band as the workspace topbar — anywhere
              not on a button drags the window. */}
          <header
            data-tauri-drag-region
            className="flex shrink-0 items-center justify-between gap-2 border-b border-line px-5 pb-3.5 pt-9"
          >
            <div className="flex h-9 items-center gap-0.5">
              <RailViewButton
                icon={UsersIcon}
                label={t("Runners")}
                active={railView === "runners"}
                onClick={() => setRailView("runners")}
              />
              <RailViewButton
                icon={Info}
                label={t("Mission detail")}
                active={railView === "meta"}
                onClick={() => setRailView("meta")}
              />
            </div>
            <div className="flex h-9 items-center">
              <button
                type="button"
                onClick={() => setRailOpen(false)}
                title={t("Collapse panel")}
                aria-label={t("Collapse panel")}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded border border-transparent text-fg-2 transition-colors hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg focus:outline-none"
              >
                <PanelRight aria-hidden className="h-4 w-4" />
              </button>
            </div>
          </header>
          <div className="flex min-h-0 flex-1 flex-col pt-5">
            {railView === "runners" ? (
              <RunnersRail
                sessions={sessions}
                selectedSessionId={activeTab === "feed" ? null : activeTab}
                status={runnerStatusMap}
                leadHandle={leadHandle}
                onOpenPty={onOpenPty}
              />
            ) : mission ? (
              <MissionMetaPanel
                mission={mission}
                crew={crew}
                missionGoal={missionGoal}
              />
            ) : null}
          </div>
        </div>
        {/* Drag-to-resize strip on the left edge — mirrors the left
            sidebar's right-edge handle. Inert when collapsed. */}
        <div
          onPointerDown={railOpen ? onRailResizeStart : undefined}
          title={railOpen ? t("Drag to resize") : undefined}
          className={
            railOpen
              ? "absolute left-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-accent/40"
              : "absolute left-0 top-0 z-20 h-full w-1 bg-transparent"
          }
        />
      </aside>
      <MissionResetConfirm
        open={resetConfirmOpen && mission !== null}
        missionTitle={mission?.title ?? ""}
        onClose={() => setResetConfirmOpen(false)}
        onConfirm={() => void resetMission()}
      />
    </div>
  );
}

/// A slot's PTY pane. Three states map to Pencil nodes:
///
///   - running: live xterm (no overlay)
///   - stopped/crashed: dimmed xterm + bottom-dock card (`vS5ce` →
///     `jMJmx` for the slot variant). User can Resume.
///   - resuming: blank xterm + centered cyan pill (`GZhHO` →
///     `a3c7p`). Set either by the per-slot Resume button OR by the
///     mission-level Resume button (parent passes `forcedResuming`).
///
/// Keeping the xterm mounted across states preserves the scrollback,
/// so a user reading the prior turn before resuming sees no flash.
function SlotPtyPane({
  session,
  active,
  forcedResuming,
  anySessionLive,
  onError,
  onResumeMission,
  onArchiveMission,
  registerTerminal,
}: {
  session: SessionRow;
  active: boolean;
  /** True when the parent's "Resume mission" button is iterating
   *  through every slot. Drives the resuming overlay in this pane. */
  forcedResuming?: boolean;
  /** True iff at least one other slot in this mission is still
   *  alive — selects which paused-card subtitle the shared
   *  MissionPausedCard renders. Plumbed through so the slot card
   *  and the mission-feed card show identical chrome. */
  anySessionLive: boolean;
  onError: (e: string) => void;
  /** Mission-wide resume callback. The slot pane's overlay no longer
   *  resumes a single PTY in isolation — a partial mission state
   *  isn't a valid run, so any "Resume" affordance respawns every
   *  stopped slot via the parent. */
  onResumeMission: () => void | Promise<void>;
  /** Mission-wide archive callback. Slot-level archive would orphan
   *  the slot, so the paused-slot card escalates to mission archive
   *  the same way it escalates to mission resume. */
  onArchiveMission: () => void | Promise<void>;
  /** Hand the parent a handle to this slot's xterm so it can measure
   *  cols/rows before the resume RPC and avoid the 80×24 default. */
  registerTerminal: (handle: RunnerTerminalHandle | null) => void;
}) {
  const t = useT();
  const resuming = !!forcedResuming;
  const dead = session.status !== "running";
  // Per-slot "warming up" overlay: when the pane first mounts for a
  // genuinely fresh slot, the agent CLI (claude-code / codex) takes
  // a beat to paint its banner. Show the cyan pill over the
  // otherwise-blank xterm canvas for at least 1s so the user doesn't
  // stare at an empty pane wondering whether the spawn succeeded.
  // Mirrors the resume dismissal dance (first-output + idle +
  // min-visible).
  //
  // Gated on `isFreshSpawn(session.started_at)` so that
  // navigating into an old mission — slot PTYs running for hours,
  // agent long-since painted — drops straight into the live
  // terminal instead of flashing the pill over already-rendered
  // content.
  const [starting, setStarting] = useState<boolean>(
    () => !dead && isFreshSpawn(session.started_at),
  );
  useEffect(() => {
    if (!starting) return;
    const STARTING_MIN_VISIBLE_MS = 1000;
    const STARTING_IDLE_DEBOUNCE_MS = 400;
    const STARTING_HARD_TIMEOUT_MS = 10_000;
    const startTs = performance.now();
    const targetId = session.id;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    let idleTimer: number | null = null;
    const finish = () => {
      if (!cancelled) setStarting(false);
    };
    const scheduleIdleTimer = () => {
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      const elapsed = performance.now() - startTs;
      const delay = Math.max(
        STARTING_IDLE_DEBOUNCE_MS,
        STARTING_MIN_VISIBLE_MS - elapsed,
      );
      idleTimer = window.setTimeout(finish, delay);
    };
    scheduleIdleTimer();
    const hardTimeout = window.setTimeout(finish, STARTING_HARD_TIMEOUT_MS);
    // Snapshot fast-path: mission_start can take several seconds to
    // return (slots behind the claude-code launch gate spawn
    // serially) so by the time this slot pane mounts, the lead's
    // TUI has often already emitted the bracketed-paste / alt-screen
    // ready signal. The live listener missed those bytes; the
    // snapshot still carries them via output_buffers.
    void api.session
      .outputSnapshot(targetId)
      .then((snapshot) => {
        if (cancelled) return;
        if (snapshot.some((ev) => chunkIndicatesTuiReady(ev.data))) {
          finish();
        }
      })
      .catch(() => {
        // Best-effort; live listener still applies.
      });
    void listen<{ session_id: string; data: string }>(
      "session/output",
      (event) => {
        if (event.payload.session_id !== targetId) return;
        // Fast-path: claude-code / codex enable bracketed paste mode
        // (`\x1b[?2004h`) as soon as their TUI is wired up to accept
        // input, before the first-turn reply streams. Without this,
        // slot pills hang until the agent's reply finishes — the
        // user notices it most in missions where multiple panes are
        // on screen at once.
        if (chunkIndicatesTuiReady(event.payload.data)) {
          finish();
          return;
        }
        scheduleIdleTimer();
      },
    ).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (idleTimer !== null) window.clearTimeout(idleTimer);
      window.clearTimeout(hardTimeout);
      unlisten?.();
    };
  }, [starting, session.id]);
  // If the slot dies before the pill clears, drop the pill so the
  // Session ended card takes over without fighting the pill for the
  // overlay slot.
  useEffect(() => {
    if (dead && starting) setStarting(false);
  }, [dead, starting]);

  // Mission slot pane omits the Archive option — archiving a slot's
  // session row would orphan the slot in the workspace. Mission-level
  // archive lives in the topbar kebab.
  //
  // Pane opacity:
  //   - resuming/starting: 0 (canvas hidden, the pill carries the visual)
  //   - dead but not resuming: 45% (the user can read scrollback)
  //   - running: 100%
  const paneOpacity =
    resuming || starting ? "opacity-0" : dead ? "opacity-45" : "";
  // Padding frame around the xterm canvas tracks the active terminal
  // palette's bg, same as in RunnerChat — keeps the frame and the
  // canvas in lockstep across theme switches.
  const terminalBg = useTerminalBg();
  return (
    <div className="relative flex flex-1 min-h-0 flex-col">
      <div
        style={{ backgroundColor: terminalBg }}
        className={`flex flex-1 min-h-0 p-3 transition-opacity ${paneOpacity}`}
      >
        <RunnerTerminal
          ref={registerTerminal}
          sessionId={session.id}
          runnerRuntime={session.runtime}
          onError={onError}
          active={active && !resuming && !starting}
          disabled={dead || resuming || starting}
        />
      </div>
      {resuming ? (
        <ResumingOverlay />
      ) : starting ? (
        <StartingOverlay label={t("Starting chat…")} />
      ) : dead ? (
        <MissionPausedCard
          anySessionLive={anySessionLive}
          onResumeMission={onResumeMission}
          onArchiveMission={onArchiveMission}
        />
      ) : null}
    </div>
  );
}

function Pane({
  active,
  children,
}: {
  active: boolean;
  children: React.ReactNode;
}) {
  // Pane wraps both the feed and the terminal slots. Background is
  // `bg-panel` so the feed reads as a tinted "page" with the white
  // event cards (`bg-bg`) lifting off it. Terminal slots cover this bg
  // with their own terminal palette wrapper.
  return (
    <div
      className={`absolute inset-0 flex-col bg-panel ${
        active ? "flex" : "hidden"
      }`}
    >
      {children}
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
  shortcut,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  shortcut?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`-mb-px flex items-center border-b-2 px-3.5 py-2.5 text-[13px] leading-none transition-colors ${
        active
          ? "border-accent font-medium text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      {children}
      {shortcut ? (
        <span className="ml-3 font-mono text-[11px] font-normal leading-none text-fg-3">
          {shortcut}
        </span>
      ) : null}
    </button>
  );
}

/// PTY tab — terminal icon + handle + close `×`. Closing snaps the
/// active tab back to feed if it was the closed one (handled in the
/// parent's onClose). Keep the click target on the tab itself; the ×
/// stops propagation so closing doesn't also activate.
function PtyTabButton({
  handle,
  active,
  onClick,
  onClose,
  shortcut,
}: {
  handle: string;
  active: boolean;
  onClick: () => void;
  onClose: () => void;
  shortcut?: string;
}) {
  const t = useT();
  return (
    <button
      type="button"
      onClick={onClick}
      title={`@${handle}`}
      className={`-mb-px flex items-center gap-2 border-b-2 px-3.5 py-2.5 text-[13px] leading-none transition-colors ${
        active
          ? "border-accent font-medium text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      <Terminal aria-hidden className="h-3 w-3 shrink-0" />
      <span className="max-w-[140px] truncate font-mono">@{handle}</span>
      {shortcut ? (
        <span className="ml-1 font-mono text-[11px] font-normal leading-none text-fg-3">
          {shortcut}
        </span>
      ) : null}
      <span
        role="button"
        aria-label={t("Close @{handle} tab", { handle })}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className="inline-flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center rounded text-fg-3 hover:bg-raised hover:text-fg"
      >
        <X aria-hidden className="h-3 w-3" />
      </span>
    </button>
  );
}

/// Topbar overflow menu for the mission. Same Pin/Rename/Archive shape
/// as the sidebar's mission context menu — both surfaces converge on
/// the design's `EWpGa` popover. Pin and Rename render as disabled
/// placeholders until those actions land; Archive fires the destructive
/// `mission_archive` path the parent component owns.
function MissionKebab({
  pinned,
  open,
  onToggle,
  onClose,
  onPin,
  onRename,
  onReset,
  onArchive,
}: {
  pinned: boolean;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
  onReset: () => void;
  onArchive: () => void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
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
  }, [open, onClose]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label={t("Mission actions")}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={onToggle}
        className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-transparent text-fg-2 transition-colors hover:border-line hover:bg-raised hover:text-fg"
      >
        <MoreHorizontal aria-hidden className="h-4 w-4" />
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-50 mt-1.5 flex w-40 flex-col gap-px rounded-lg border border-line bg-raised p-1.5 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
        >
          <KebabItem
            icon={pinned ? PinOff : Pin}
            label={pinned ? t("Unpin") : t("Pin")}
            onClick={onPin}
          />
          <KebabItem icon={SquarePen} label={t("Rename")} onClick={onRename} />
          <KebabItem icon={RotateCcw} label={t("Reset")} onClick={onReset} />
          <KebabItem
            icon={Archive}
            label={t("Archive")}
            onClick={onArchive}
            danger
          />
        </div>
      ) : null}
    </div>
  );
}

function KebabItem({
  icon: Icon,
  label,
  onClick,
  disabled,
  danger,
}: {
  icon: typeof Archive;
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
      <Icon aria-hidden className={`h-3.5 w-3.5 ${danger ? "text-danger" : "text-fg"}`} />
      <span>{label}</span>
    </button>
  );
}

function RailViewButton({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      aria-pressed={active}
      className={`flex h-7 w-7 cursor-pointer items-center justify-center rounded border border-transparent transition-colors focus:outline-none ${
        active
          ? "bg-sidebar-selected/60 text-fg"
          : "text-fg-2 hover:bg-sidebar-selected/60 hover:text-fg focus:bg-sidebar-selected/60 focus:text-fg"
      }`}
    >
      <Icon aria-hidden className="h-3.5 w-3.5" />
    </button>
  );
}

function formatRelativeTime(iso: string, t: TFn): string {
  try {
    const d = new Date(iso);
    const diffMs = Date.now() - d.getTime();
    const minutes = Math.floor(diffMs / 60000);
    if (minutes < 1) return t("just now");
    if (minutes < 60) return t("{n}m ago", { n: minutes });
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return t("{n}h ago", { n: hours });
    const days = Math.floor(hours / 24);
    return t("{n}d ago", { n: days });
  } catch {
    return iso;
  }
}

/// "Mission paused" card — shown in both the mission-feed surface
/// (when any slot is stopped) and inside a stopped slot's PTY pane.
/// Semantically the same state in both places: the mission isn't
/// running. Extracted so the two call sites can't drift (issue
/// #173). `anySessionLive` swaps the subtitle between the partial-
/// mission and all-paused variants; both call sites already compute
/// it at the parent so we just plumb it down to the slot.
function MissionPausedCard({
  anySessionLive,
  onResumeMission,
  onArchiveMission,
}: {
  anySessionLive: boolean;
  onResumeMission: () => void | Promise<void>;
  onArchiveMission: () => void | Promise<void>;
}) {
  const t = useT();
  return (
    <SessionEndedOverlay
      status="stopped"
      resumable
      title={t("Mission paused")}
      subtitle={
        anySessionLive
          ? t(
              "One or more slots are paused. Resume the mission to respawn every paused slot — partial-mission states aren't a valid run.",
            )
          : t(
              "All slots are paused. Resume to respawn every slot and pick up the conversation — the event log is preserved.",
            )
      }
      resumeLabel={t("Resume")}
      onResume={() => void onResumeMission()}
      archiveLabel={t("Archive")}
      onArchive={() => void onArchiveMission()}
      variant="inline"
    />
  );
}
