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
import {
  Archive,
  Flag,
  MoreHorizontal,
  Pin,
  PinOff,
  PanelRightClose,
  PanelRightOpen,
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
  // Destructive: confirm before firing.
  const archiveMission = useCallback(async () => {
    if (!mission) return;
    if (!confirm("Archive this mission? This ends it permanently — sessions cannot be resumed afterward.")) return;
    try {
      const next = await api.mission.archive(mission.id);
      setMission(next);
      const rows = await api.session.list(mission.id);
      setSessions(rows);
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
    try {
      for (const s of sessions) {
        if (s.status === "running") continue;
        await api.session.resume(s.id, null, null);
      }
      const rows = await api.session.list(mission.id);
      setSessions(rows);
    } catch (e) {
      setError(String(e));
    } finally {
      setResumingAll(false);
    }
  }, [mission, sessions]);

  const allSessionsLive =
    sessions.length > 0 && sessions.every((s) => s.status === "running");
  const anySessionStopped =
    sessions.length > 0 && sessions.some((s) => s.status !== "running");

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
            <h1 className="truncate text-[14px] font-semibold leading-tight text-fg">
              {mission?.title ?? "…"}
            </h1>
            <span className="truncate text-[11px] leading-tight text-fg-3">
              {sessions.length} runner{sessions.length === 1 ? "" : "s"}
              {startedAt ? ` · started ${startedAt}` : ""}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {mission ? (() => {
            // Display status is derived: a `running` mission with no
            // live PTYs reads as "stopped" so the badge matches the
            // Resume button next to it. Mission row state
            // (`mission.status`) is still authoritative for backend
            // gating; only the visual label is derived.
            const display: "running" | "stopped" | "archived" | "aborted" =
              mission.status === "running"
                ? allSessionsLive
                  ? "running"
                  : "stopped"
                : mission.status === "completed"
                  ? "archived"
                  : "aborted";
            // Smaller pill matching design `M5Kohk` — tighter padding,
            // pure rounded ends, slightly subdued bg tints.
            const pillClass =
              display === "running"
                ? "bg-accent/15 text-accent"
                : display === "aborted"
                  ? "bg-danger/15 text-danger"
                  : "bg-raised text-fg-2";
            const dotClass =
              display === "running"
                ? "bg-accent"
                : display === "aborted"
                  ? "bg-danger"
                  : "bg-fg-3";
            return (
              <span
                className={`inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium ${pillClass}`}
              >
                <span
                  className={`inline-flex h-1.5 w-1.5 rounded-full ${dotClass}`}
                />
                {display}
              </span>
            );
          })() : null}
          {mission?.status === "running" ? (
            <>
              {anySessionStopped ? (
                <button
                  type="button"
                  onClick={() => void resumeMission()}
                  disabled={resumingAll}
                  className="inline-flex items-center gap-1.5 rounded-md border border-accent/40 bg-accent/10 px-2.5 py-1 text-[11px] font-semibold text-accent hover:border-accent disabled:cursor-default disabled:opacity-60"
                  title="Respawn every stopped slot in this mission"
                >
                  {resumingAll ? "Resuming…" : "Resume"}
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
            {openTabs
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

            {openTabs
              .map((tabId) => sessions.find((s) => s.id === tabId))
              .filter((s): s is SessionRow => s !== undefined)
              .map((s) => (
                <Pane key={s.id} active={activeTab === s.id}>
                  <SlotPtyPane
                    session={s}
                    active={activeTab === s.id}
                    onError={setError}
                    onResumed={async () => {
                      const rows = await api.session.list(mission.id);
                      setSessions(rows);
                    }}
                  />
                </Pane>
              ))}
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
    </div>
  );
}

/// A slot's PTY pane. Running sessions render an xterm; stopped/crashed
/// rows show a centered Resume button that calls session_resume (the
/// backend respawns the same row, preserving agent_session_key so the
/// agent CLI picks up the same conversation thread).
function SlotPtyPane({
  session,
  active,
  onError,
  onResumed,
}: {
  session: SessionRow;
  active: boolean;
  onError: (e: string) => void;
  onResumed: () => void | Promise<void>;
}) {
  const [resuming, setResuming] = useState(false);

  if (session.status === "running") {
    return (
      <div className="flex flex-1 min-h-0 p-3">
        <RunnerTerminal
          sessionId={session.id}
          onError={onError}
          active={active}
        />
      </div>
    );
  }

  const onResume = async () => {
    if (resuming) return;
    setResuming(true);
    try {
      await api.session.resume(session.id, null, null);
      await onResumed();
    } catch (e) {
      onError(String(e));
    } finally {
      setResuming(false);
    }
  };

  const stoppedReason =
    session.status === "crashed"
      ? "@" + session.handle + " crashed."
      : "@" + session.handle + " is stopped.";

  return (
    <div className="flex flex-1 min-h-0 items-center justify-center p-6">
      <div className="flex max-w-sm flex-col items-center gap-3 rounded-lg border border-line bg-panel px-6 py-8 text-center">
        <span className="text-sm font-medium text-fg">{stoppedReason}</span>
        <span className="text-xs text-fg-3">
          Resume respawns the same session — claude-code reattaches via
          its session UUID; codex reuses the captured rollout.
        </span>
        <button
          type="button"
          onClick={() => void onResume()}
          disabled={resuming}
          className="mt-1 inline-flex items-center justify-center rounded border border-line-strong bg-bg px-3 py-1.5 text-sm font-medium text-fg transition-colors hover:bg-raised disabled:cursor-default disabled:opacity-60"
        >
          {resuming ? "Resuming…" : "Resume @" + session.handle}
        </button>
      </div>
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
  onArchive,
}: {
  pinned: boolean;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onPin: () => void;
  onRename: () => void;
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
