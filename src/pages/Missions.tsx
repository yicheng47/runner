// Missions list (C11) — entrypoint for the workspace.
//
// Active / Past tabs split on `mission.status === "running"`. Each row
// shows crew + title + status pill + when-it-started, plus a small flag
// when the router has at least one pending `human_question` card —
// the only "needs your attention" signal we surface in v0.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";

import { api } from "../lib/api";
import type {
  AppendedEvent,
  MissionSummary,
} from "../lib/types";
import { Button } from "../components/ui/Button";
import { EmptyStateCard } from "../components/EmptyStateCard";
import { StartMissionModal } from "../components/StartMissionModal";

type Tab = "active" | "past";

export default function Missions() {
  const [missions, setMissions] = useState<MissionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("active");
  const [creating, setCreating] = useState(false);
  const navigate = useNavigate();

  const refresh = useCallback(async () => {
    try {
      setError(null);
      const rows = await api.mission.listSummary();
      setMissions(rows);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Refresh whenever a workspace appends a router-relevant event so the
  // pending-ask flag reflects reality without polling. Cheap: the
  // backend join is a small list query plus an O(N) registry lookup.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<AppendedEvent>("event/appended", (msg) => {
      const t = msg.payload.event.type;
      if (
        t === "ask_human" ||
        t === "human_question" ||
        t === "human_response" ||
        t === "mission_stopped"
      ) {
        void refresh();
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
  }, [refresh]);

  const { active, past } = useMemo(() => {
    const active: MissionSummary[] = [];
    const past: MissionSummary[] = [];
    for (const m of missions) {
      (m.status === "running" ? active : past).push(m);
    }
    return { active, past };
  }, [missions]);

  const visible = tab === "active" ? active : past;

  return (
    <>
      <div className="flex flex-1 flex-col overflow-y-auto">
        <div className="flex w-full flex-1 flex-col gap-6 px-8 py-8">
          <header className="flex items-center justify-between gap-4">
            <div className="flex flex-col gap-1">
              <h1 className="text-2xl font-bold tracking-tight text-fg">
                Missions
              </h1>
              <p className="text-sm text-fg-2">
                Live runs of your crews. Click a row to open its workspace.
              </p>
            </div>
            <Button variant="primary" onClick={() => setCreating(true)}>
              + Start mission
            </Button>
          </header>

          <div className="flex items-center gap-1 border-b border-line">
            <TabButton active={tab === "active"} onClick={() => setTab("active")}>
              Active
              <Counter count={active.length} />
            </TabButton>
            <TabButton active={tab === "past"} onClick={() => setTab("past")}>
              Past
              <Counter count={past.length} muted />
            </TabButton>
          </div>

          {error ? (
            <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
              {error}
            </div>
          ) : null}

          {loading ? (
            <div className="text-sm text-fg-2">Loading…</div>
          ) : visible.length === 0 ? (
            <EmptyStateCard
              icon={<MissionIcon />}
              title={tab === "active" ? "No live missions" : "No past missions"}
              description={
                tab === "active"
                  ? "Start one from a crew to spawn its runners and open the workspace."
                  : "Stopped or completed missions land here."
              }
              action={
                tab === "active" ? (
                  <Button variant="primary" onClick={() => setCreating(true)}>
                    + Start mission
                  </Button>
                ) : null
              }
            />
          ) : (
            <div className="flex flex-col gap-3">
              {visible.map((m) => (
                <MissionRow
                  key={m.id}
                  item={m}
                  onOpen={() => navigate(`/missions/${m.id}`)}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      <StartMissionModal
        open={creating}
        onClose={() => setCreating(false)}
        onStarted={(mission) => {
          setCreating(false);
          void refresh();
          navigate(`/missions/${mission.id}`);
        }}
      />
    </>
  );
}

function MissionRow({
  item,
  onOpen,
}: {
  item: MissionSummary;
  onOpen: () => void;
}) {
  const dotClass =
    item.status === "running"
      ? "bg-accent"
      : item.status === "aborted"
        ? "bg-danger"
        : "bg-fg-3";
  const statusPill =
    item.status === "running"
      ? "text-accent"
      : item.status === "aborted"
        ? "text-danger"
        : "text-fg-2";
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      className="group flex cursor-pointer items-center justify-between gap-4 rounded-lg border border-line bg-panel p-4 transition-colors hover:border-line-strong focus:outline-none focus-visible:border-fg-3"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span
            className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${dotClass}`}
            title={item.status}
          />
          <span className="truncate text-[14px] font-semibold text-fg">
            {item.title}
          </span>
          {item.pending_ask_count > 0 ? (
            <span
              title="Awaiting human input"
              className="rounded bg-warn/20 px-1.5 py-px text-[10px] font-bold uppercase tracking-wide text-warn"
            >
              {item.pending_ask_count} pending
            </span>
          ) : null}
        </div>
        <div className="flex items-center gap-2 text-[11px] text-fg-3">
          <span className="font-mono text-fg-2">
            {item.crew_name || "(crew deleted)"}
          </span>
          <span>·</span>
          <span className={statusPill}>{item.status}</span>
          <span>·</span>
          <span>started {formatRelativeTime(item.started_at)}</span>
          {item.stopped_at ? (
            <>
              <span>·</span>
              <span>stopped {formatRelativeTime(item.stopped_at)}</span>
            </>
          ) : null}
        </div>
      </div>
      <span className="shrink-0 text-[12px] text-fg-3 group-hover:text-accent">
        Open →
      </span>
    </div>
  );
}

function MissionIcon() {
  // Simple flag glyph — purely decorative on the empty state.
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="4" y1="22" x2="4" y2="15" />
      <path d="M4 15a5 5 0 0 1 5-5 5 5 0 0 0 5-5 5 5 0 0 1 5-5v9a5 5 0 0 1-5 5 5 5 0 0 0-5 5 5 5 0 0 1-5 5z" />
    </svg>
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
      className={`-mb-px flex cursor-pointer items-center gap-1.5 border-b-2 px-3 py-2 text-[12px] font-medium transition-colors ${
        active
          ? "border-accent text-fg"
          : "border-transparent text-fg-2 hover:text-fg"
      }`}
    >
      {children}
    </button>
  );
}

function Counter({ count, muted }: { count: number; muted?: boolean }) {
  return (
    <span
      className={`rounded px-1.5 py-px text-[10px] font-semibold ${
        muted
          ? "bg-raised text-fg-3"
          : count > 0
            ? "bg-accent/15 text-accent"
            : "bg-raised text-fg-3"
      }`}
    >
      {count}
    </span>
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
