// Right-rail variant for the mission workspace — read-only display of
// the mission id, effective mission goal (snapshotted into the
// `mission_goal` event at mission_start), `mission.cwd`, the crew
// handle (link), and a relative-time "started" row. Toggle into this
// view via the icon strip in the rail header.
//
// Editing is intentionally not in v1: goal + cwd are baked into the
// lead's launch prompt at `mission_start`, so post-start edits don't
// reach in-flight agents until the next spawn/resume. Display first;
// add edit affordances once the "applies on resume" UX is settled.

import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { Clock3, Users } from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { CopyValueButton } from "./ui/CopyValueButton";
import type { Crew, Mission } from "../lib/types";

interface MissionMetaPanelProps {
  mission: Mission;
  crew: Crew | null;
  /** Effective goal text snapshotted at mission_start. Read from the
   *  replayed `mission_goal` event so editing the crew default after
   *  launch doesn't drift this display from what the agents received.
   *  `null` while events are still loading. */
  missionGoal: string | null;
}

export function MissionMetaPanel({
  mission,
  crew,
  missionGoal,
}: MissionMetaPanelProps) {
  const cwd = mission.cwd;

  // Re-render the relative "started X ago" row every 60s so the value
  // stays current as the mission ages — matches the topbar's behavior.
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 60_000);
    return () => clearInterval(t);
  }, []);

  const revealCwd = () => {
    if (!cwd) return;
    void revealItemInDir(cwd).catch((e) => {
      console.error("MissionMetaPanel: revealItemInDir failed", e);
    });
  };

  return (
    <div className="flex flex-1 min-h-0 flex-col gap-4 overflow-y-auto px-5 pb-5">
      <div className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
        Mission detail
      </div>

      <Section label="Mission ID">
        <span className="flex min-w-0 items-start gap-1.5">
          <span className="min-w-0 flex-1 break-all font-mono text-[11px] text-fg-2">
            {mission.id}
          </span>
          <CopyValueButton value={mission.id} label="Copy mission ID" />
        </span>
      </Section>

      <Section label="Goal">
        {missionGoal === null ? (
          <p className="text-[12px] italic text-fg-3">Loading…</p>
        ) : missionGoal ? (
          <p className="whitespace-pre-wrap break-words text-[12px] leading-[1.5] text-fg">
            {missionGoal}
          </p>
        ) : (
          <p className="text-[12px] italic text-fg-3">No goal set.</p>
        )}
      </Section>

      <Section label="Working dir">
        {cwd ? (
          <button
            type="button"
            onClick={revealCwd}
            title="Reveal in Finder"
            className="w-full cursor-pointer rounded-md border border-line bg-bg px-2.5 py-2 text-left font-mono text-[11px] text-fg break-all transition-colors hover:border-line-strong"
          >
            {cwd}
          </button>
        ) : (
          <p className="text-[12px] italic text-fg-3">No cwd set.</p>
        )}
      </Section>

      <Section label="Crew">
        <Link
          to={`/crews/${mission.crew_id}`}
          className="flex items-center gap-2 text-[12px] font-medium text-accent hover:underline"
        >
          <Users aria-hidden className="h-3 w-3 text-fg-2" />
          <span className="truncate">{crew?.name ?? "…"}</span>
        </Link>
      </Section>

      <Section label="Started">
        <div className="flex items-center gap-2 text-[12px] text-fg">
          <Clock3 aria-hidden className="h-3 w-3 text-fg-2" />
          <span>{formatRelativeTime(mission.started_at)}</span>
        </div>
      </Section>

      <div className="h-px w-full bg-line" />
    </div>
  );
}

function Section({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-[10px] font-semibold uppercase tracking-[0.1em] text-fg-3">
        {label}
      </div>
      {children}
    </div>
  );
}

function formatRelativeTime(iso: string): string {
  try {
    const d = new Date(iso);
    const diffMs = Date.now() - d.getTime();
    const minutes = Math.floor(diffMs / 60000);
    if (minutes < 1) return "just now";
    if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
    const days = Math.floor(hours / 24);
    return `${days} day${days === 1 ? "" : "s"} ago`;
  } catch {
    return iso;
  }
}
