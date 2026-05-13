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
  MoreHorizontal,
  Pin,
  PinOff,
  PanelRightClose,
  PanelRightOpen,
  RotateCcw,
  Square,
  SquarePen,
  Terminal,
  X,
} from "lucide-react";

import { api, type SessionRow } from "../lib/api";
import type {
  AppendedEvent,
  Event,
  HumanQuestionPayload,
  Mission,
  WarningEvent,
} from "../lib/types";
import { EventFeed } from "../components/EventFeed";
import { MissionInput } from "../components/MissionInput";
import { MissionResetConfirm } from "../components/MissionResetConfirm";
import { RunnersRail } from "../components/RunnersRail";
import { RunnerTerminal } from "../components/RunnerTerminal";
import {
  ArchivingOverlay,
  ResumingOverlay,
  SessionEndedOverlay,
} from "../components/SessionEndedOverlay";
import {
  markArchivingMission,
  unmarkArchivingMission,
  useArchivingMission,
} from "../lib/archivingState";

export default function MissionWorkspace() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const [mission, setMission] = useState<Mission | null>(null);
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
        // Auto-open every slot's PTY tab on mount. The user can close
        // individual tabs via the × on each tab; if they close them
        // all and re-mount, the mount path opens them again.
        setOpenTabs(ss.map((s) => s.id));
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
    const next = window.prompt("Rename mission", mission.title);
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
      setOpenTabs(rows.map((s) => s.id));
      setResetConfirmOpen(false);
    } catch (e) {
      setError(String(e));
    }
  }, [mission]);

  // Resume all = iterate stopped/crashed sessions and respawn each.
  // Hits the same `session_resume` path the per-slot Resume button
  // uses; just saves clicks when every slot needs to come back.
  const [resumingAll, setResumingAll] = useState(false);
  const resumeMission = useCallback(async () => {
    if (!mission) return;
    setResumingAll(true);
    let firstErr: string | null = null;
    try {
      // Best-effort over every stopped slot. Don't bail on the first
      // failure — earlier slots may have already resumed, and the
      // user wants the UI to reflect whatever actually came up.
      // Errors are collected and surfaced after the refresh.
      for (const s of sessions) {
        if (s.status === "running") continue;
        try {
          await api.session.resume(s.id, null, null);
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

  const onOpenPty = useCallback((sessionId: string) => {
    setOpenTabs((prev) =>
      prev.includes(sessionId) ? prev : [...prev, sessionId],
    );
    setActiveTab(sessionId);
  }, []);

  const onCloseTab = useCallback((sessionId: string) => {
    setOpenTabs((prev) => prev.filter((id) => id !== sessionId));
    setActiveTab((prev) => (prev === sessionId ? "feed" : prev));
  }, []);

  const handles = sessions.map((s) => s.handle);
  const startedAt = mission ? formatRelativeTime(mission.started_at) : "";
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

  return (
    // flex-row outer so the right rail becomes a top-level sibling
    // of the main column. The rail then spans the full workspace
    // height, with its own header that lines up with the topbar
    // across the divider — same layout shape as RunnerChat.
    <div className="flex h-full flex-1 flex-row bg-bg">
      <div className="flex min-w-0 flex-1 flex-col">
      <header className="flex items-center justify-between gap-4 border-b border-line bg-panel px-6 pb-3.5 pt-9">
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
                Mission
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
                        ? "bg-[#0F1E26] text-[#39E5FF]"
                        : "bg-raised text-fg-2";
                const dotClass =
                  display === "running"
                    ? "bg-accent"
                    : display === "aborted"
                      ? "bg-danger"
                      : display === "resuming"
                        ? "bg-[#39E5FF]"
                        : "bg-fg-3";
                return (
                  <span
                    className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${pillClass}`}
                  >
                    <span
                      className={`inline-flex h-1.5 w-1.5 rounded-full ${dotClass}`}
                    />
                    {display === "resuming" ? "resuming…" : display}
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
                  Archived · read-only
                </span>
              ) : null}
            </div>
            <span className="truncate text-[11px] leading-tight text-fg-3">
              {sessions.length} runner{sessions.length === 1 ? "" : "s"}
              {startedAt ? ` · started ${startedAt}` : ""}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {mission?.status === "running" && !resumingAll ? (
            <>
              {anySessionStopped ? (
                <button
                  type="button"
                  onClick={() => void resumeMission()}
                  className="inline-flex items-center gap-1.5 rounded-md border border-accent/40 bg-accent/10 px-2.5 py-1 text-[11px] font-semibold text-accent hover:border-accent"
                  title="Respawn every stopped slot in this mission"
                >
                  Resume
                </button>
              ) : null}
              {allSessionsLive ? (
                <button
                  type="button"
                  onClick={() => void stopMission()}
                  className="inline-flex items-center gap-1.5 rounded-md border border-line bg-raised px-2.5 py-1 text-[11px] font-semibold text-fg hover:border-line-strong"
                  title="Kill all PTYs; mission stays running so you can Resume"
                >
                  <Square aria-hidden className="h-3 w-3" />
                  Stop
                </button>
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
              title="Open runners panel"
              aria-label="Open runners panel"
              className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-transparent text-fg-2 transition-colors hover:border-line hover:bg-raised hover:text-fg"
            >
              <PanelRightOpen aria-hidden className="h-4 w-4" />
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
            Dismiss
          </button>
        </div>
      ) : null}

      {loading || !mission ? (
        <div className="px-8 py-8 text-sm text-fg-2">Loading mission…</div>
      ) : (
        <div className="flex flex-1 min-h-0 flex-col">
          <div className="flex h-[38px] items-end gap-1 border-b border-line bg-panel px-6">
            <TabButton
              active={activeTab === "feed"}
              onClick={() => setActiveTab("feed")}
            >
              feed
            </TabButton>
            {/* Archived missions render feed only — skip the per-PTY
                tabs so no xterm canvas ever mounts. The Pane block
                below applies the same gate. */}
            {!isArchived
              ? openTabs
                  .map((tabId) => sessions.find((s) => s.id === tabId))
                  .filter((s): s is SessionRow => s !== undefined)
                  .map((s) => (
                    <PtyTabButton
                      key={s.id}
                      handle={s.handle}
                      active={activeTab === s.id}
                      onClick={() => setActiveTab(s.id)}
                      onClose={() => onCloseTab(s.id)}
                    />
                  ))
              : null}
          </div>

          <div className="relative flex flex-1 min-h-0 flex-col">
            {/* All panes stay mounted so xterm's in-memory scrollback
                survives tab switches. Inactive panes use display:none:
                that keeps the React/xterm instances alive while making
                the visible session unambiguous. The terminal activation
                effect refits + repaints after the pane is shown. */}
            <Pane active={activeTab === "feed"}>
              <EventFeed
                missionId={mission.id}
                events={events}
                resolvedAsks={resolvedAsks}
                askersByQuestion={askersByQuestion}
                onError={setError}
              />
              <MissionInput
                missionId={mission.id}
                leadHandle={leadHandle}
                handles={handles}
                disabled={mission.status !== "running" || !allSessionsLive}
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
              !archivingMission ? (
                <>
                  {/* Backdrop only — sits behind the inline-variant
                      card so the feed dims and reads as paused
                      without moving the card off its original
                      bottom anchor. */}
                  <div className="pointer-events-none absolute inset-0 z-0 bg-bg/70 backdrop-blur-sm" />
                  <SessionEndedOverlay
                    status="stopped"
                    resumable
                    title="Mission paused"
                    subtitle={
                      anySessionLive
                        ? "One or more slots stopped. Resume the mission to respawn every stopped slot — partial-mission states aren't a valid run."
                        : "All slots are stopped. Resume to respawn every slot and pick up the conversation — the event log is preserved."
                    }
                    resumeLabel="Resume mission"
                    onResume={() => void resumeMission()}
                    variant="inline"
                  />
                </>
              ) : null}
            </Pane>

            {/* Skip per-session PTY panes for archived missions so
                no xterm canvas ever mounts. The feed Pane stays
                rendered above as the only surface. */}
            {!isArchived
              ? openTabs
                  .map((tabId) => sessions.find((s) => s.id === tabId))
                  .filter((s): s is SessionRow => s !== undefined)
                  .map((s) => (
                    <Pane key={s.id} active={activeTab === s.id}>
                      <SlotPtyPane
                        session={s}
                        active={activeTab === s.id}
                        forcedResuming={resumingAll && !archivingMission}
                        onError={setError}
                        onResumeMission={() => void resumeMission()}
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
          </div>
        </div>
      )}
      </div>
      {/* Collapsible right rail — top-level sibling of the main
          column so it spans the full workspace height (header
          included). Mirrors the RunnerChat pattern: the inner w-72
          wrapper stays mounted and clipped by overflow-hidden so
          width animates without reflowing the children. */}
      <aside
        aria-hidden={!railOpen}
        className={`flex shrink-0 flex-col overflow-hidden bg-panel transition-[width,border-left-width] duration-200 ease-in-out ${
          railOpen ? "w-72 border-l border-line" : "w-0 border-l-0"
        }`}
      >
        <div className="flex h-full w-72 flex-col">
          {/* Rail header — same px-5 / pt-9 / pb-3.5 / border-b
              rhythm as the workspace topbar so the collapse button
              shares a baseline with the topbar buttons across the
              divider. */}
          <header className="flex shrink-0 items-center justify-end border-b border-line px-5 pb-3.5 pt-9">
            <div className="flex h-9 items-center">
              <button
                type="button"
                onClick={() => setRailOpen(false)}
                title="Collapse runners panel"
                aria-label="Collapse runners panel"
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-2 hover:bg-raised hover:text-fg"
              >
                <PanelRightClose aria-hidden className="h-4 w-4" />
              </button>
            </div>
          </header>
          <div className="flex min-h-0 flex-1 flex-col pt-5">
            <RunnersRail
              sessions={sessions}
              selectedSessionId={activeTab === "feed" ? null : activeTab}
              status={runnerStatusMap}
              leadHandle={leadHandle}
              onOpenPty={onOpenPty}
            />
          </div>
        </div>
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
  onError,
  onResumeMission,
}: {
  session: SessionRow;
  active: boolean;
  /** True when the parent's "Resume mission" button is iterating
   *  through every slot. Drives the resuming overlay in this pane. */
  forcedResuming?: boolean;
  onError: (e: string) => void;
  /** Mission-wide resume callback. The slot pane's overlay no longer
   *  resumes a single PTY in isolation — a partial mission state
   *  isn't a valid run, so any "Resume" affordance respawns every
   *  stopped slot via the parent. */
  onResumeMission: () => void | Promise<void>;
}) {
  const resuming = !!forcedResuming;
  const dead = session.status !== "running";

  // Mission slot pane omits the Archive option — archiving a slot's
  // session row would orphan the slot in the workspace. Mission-level
  // archive lives in the topbar kebab. `resumable` defaults to true:
  // we don't have agent_session_key on the SessionRow, but mission
  // sessions almost always carry one (claude-code self-assigns a
  // UUID on every fresh spawn) and the worst case is friendlier
  // copy than reality.
  //
  // Pane opacity:
  //   - resuming: 0 (canvas wiped, the pill carries the visual)
  //   - dead but not resuming: 45% (the user can read scrollback)
  //   - running: 100%
  const paneOpacity = resuming ? "opacity-0" : dead ? "opacity-45" : "";
  return (
    <div className="relative flex flex-1 min-h-0 flex-col">
      <div
        className={`flex flex-1 min-h-0 p-3 transition-opacity ${paneOpacity}`}
      >
        <RunnerTerminal
          sessionId={session.id}
          onError={onError}
          active={active && !dead && !resuming}
          disabled={dead || resuming}
        />
      </div>
      {resuming ? (
        <ResumingOverlay />
      ) : dead ? (
        <SessionEndedOverlay
          status={session.status}
          resumable
          title="Slot stopped"
          subtitle="This slot's PTY is closed. Resume the mission to respawn every stopped slot — partial-mission states aren't a valid run."
          resumeLabel="Resume mission"
          onResume={() => {
            void onResumeMission();
          }}
          variant="inline"
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
  return (
    <div
      className={`absolute inset-0 flex-col bg-bg ${
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
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`-mb-px border-b-2 px-3.5 py-2.5 text-[13px] transition-colors ${
        active
          ? "border-accent font-medium text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      {children}
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
}: {
  handle: string;
  active: boolean;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`-mb-px flex items-center gap-2 border-b-2 px-3.5 py-2.5 text-[13px] transition-colors ${
        active
          ? "border-accent font-medium text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      <Terminal aria-hidden className="h-3 w-3" />
      <span className="font-mono">@{handle}</span>
      <span
        role="button"
        aria-label={`Close @${handle} tab`}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className="inline-flex h-4 w-4 cursor-pointer items-center justify-center rounded text-fg-3 hover:bg-raised hover:text-fg"
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
        aria-label="Mission actions"
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
            label={pinned ? "Unpin" : "Pin"}
            onClick={onPin}
          />
          <KebabItem icon={SquarePen} label="Rename" onClick={onRename} />
          <KebabItem icon={RotateCcw} label="Reset" onClick={onReset} />
          <KebabItem icon={Archive} label="Archive" onClick={onArchive} danger />
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

function formatRelativeTime(iso: string): string {
  try {
    const d = new Date(iso);
    const diffMs = Date.now() - d.getTime();
    const minutes = Math.floor(diffMs / 60000);
    if (minutes < 1) return "just now";
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  } catch {
    return iso;
  }
}
