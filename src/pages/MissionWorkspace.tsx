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
import { useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";

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
import { RunnersRail } from "../components/RunnersRail";
import { RunnerTerminal } from "../components/RunnerTerminal";

export default function MissionWorkspace() {
  const { id } = useParams<{ id: string }>();
  const [mission, setMission] = useState<Mission | null>(null);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [events, setEvents] = useState<Event[]>([]);
  const [error, setError] = useState<string | null>(null);
  // Resume-fallback banner; non-blocking advisory (see types.ts WarningEvent).
  const [warning, setWarning] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"feed" | string>("feed"); // string = sessionId
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

        const [m, ss, evs] = await Promise.all([
          api.mission.get(id),
          api.session.list(id),
          api.mission.eventsReplay(id),
        ]);
        if (cancelled) return;
        setMission(m);
        setSessions(ss);
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

  const stopMission = useCallback(async () => {
    if (!mission) return;
    if (!confirm("Stop this mission?")) return;
    try {
      const stopped = await api.mission.stop(mission.id);
      setMission(stopped);
      const rows = await api.session.list(mission.id);
      setSessions(rows);
    } catch (e) {
      setError(String(e));
    }
  }, [mission]);

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
    setActiveTab(sessionId);
  }, []);

  const handles = sessions.map((s) => s.handle);
  const startedAt = mission ? formatRelativeTime(mission.started_at) : "";

  return (
    <div className="flex h-full flex-1 flex-col bg-bg">
      <header className="flex items-start justify-between gap-4 border-b border-line bg-panel px-8 pb-4 pt-9">
        <div className="flex flex-col gap-1 min-w-0">
          <div className="flex items-baseline gap-3 min-w-0">
            <h1 className="truncate text-[15px] font-semibold text-fg">
              {mission?.title ?? "…"}
            </h1>
            <span className="truncate text-[11px] text-fg-3">
              {sessions.length} runner{sessions.length === 1 ? "" : "s"}
              {startedAt ? ` · started ${startedAt}` : ""}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {mission ? (
            <span
              className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium ${
                mission.status === "running"
                  ? "bg-accent/10 text-accent"
                  : mission.status === "aborted"
                    ? "bg-danger/10 text-danger"
                    : "bg-raised text-fg-2"
              }`}
            >
              <span
                className={`inline-flex h-1.5 w-1.5 rounded-full ${
                  mission.status === "running"
                    ? "bg-accent"
                    : mission.status === "aborted"
                      ? "bg-danger"
                      : "bg-fg-3"
                }`}
              />
              {mission.status}
            </span>
          ) : null}
          {mission?.status === "running" ? (
            <button
              type="button"
              onClick={() => void stopMission()}
              className="rounded border border-line-strong bg-raised px-3 py-1 text-[11px] font-semibold text-fg hover:border-fg-3"
            >
              Stop
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
        <div className="flex flex-1 min-h-0">
          <div className="flex flex-1 min-w-0 flex-col">
            <div className="flex items-center gap-1 border-b border-line bg-panel px-6">
              <TabButton
                active={activeTab === "feed"}
                onClick={() => setActiveTab("feed")}
              >
                feed
              </TabButton>
              {sessions.map((s) => (
                <TabButton
                  key={s.id}
                  active={activeTab === s.id}
                  onClick={() => setActiveTab(s.id)}
                >
                  @{s.handle} pty
                </TabButton>
              ))}
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
                  disabled={mission.status !== "running"}
                  onError={setError}
                />
              </Pane>

              {sessions.map((s) => (
                <Pane key={s.id} active={activeTab === s.id}>
                  <div className="flex flex-1 min-h-0 p-3">
                    <RunnerTerminal
                      sessionId={s.id}
                      onError={setError}
                      active={activeTab === s.id}
                    />
                  </div>
                </Pane>
              ))}
            </div>
          </div>

          <RunnersRail
            sessions={sessions}
            selectedSessionId={activeTab === "feed" ? null : activeTab}
            status={runnerStatusMap}
            leadHandle={leadHandle}
            onOpenPty={onOpenPty}
          />
        </div>
      )}
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
      className={`-mb-px border-b-2 px-3 py-2 text-[12px] transition-colors ${
        active
          ? "border-accent text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      {children}
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
