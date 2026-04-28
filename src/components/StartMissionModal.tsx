// Start Mission modal (C11) — entrypoint for everything C10 renders.
//
// Crew picker is filtered to crews that look launchable (≥1 runner). The
// validation that actually rejects the mission lives on the Rust side:
// `mission_start` checks for runners + a lead and returns a clean Error
// either way; the modal surfaces that error inline so an empty-crew
// pick is recoverable without closing.

import { useEffect, useMemo, useRef, useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronRight, X } from "lucide-react";

import { api } from "../lib/api";
import type { CrewListItem, CrewRunner, Mission } from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";

interface StartMissionModalProps {
  open: boolean;
  onClose: () => void;
  /** Called once `mission_start` returns. Caller owns navigation to the
   *  workspace — keeps this component agnostic of routing. */
  onStarted: (mission: Mission) => void;
  /** Pre-pick a crew when present. Lets the Crew Detail page open the
   *  modal already scoped to its crew. */
  initialCrewId?: string | null;
}

export function StartMissionModal({
  open,
  onClose,
  onStarted,
  initialCrewId = null,
}: StartMissionModalProps) {
  const [crews, setCrews] = useState<CrewListItem[]>([]);
  const [crewId, setCrewId] = useState<string>("");
  const [crewPickerOpen, setCrewPickerOpen] = useState(false);
  const [crewRunners, setCrewRunners] = useState<CrewRunner[]>([]);
  const [title, setTitle] = useState("");
  const [goal, setGoal] = useState("");
  const [cwd, setCwd] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const crewPickerRef = useRef<HTMLDivElement | null>(null);

  // Reset every time the modal opens — otherwise stale values from the
  // previous open leak through, including a previous "this crew has no
  // runners" error that no longer applies.
  useEffect(() => {
    if (!open) return;
    setError(null);
    setSubmitting(false);
    setTitle("");
    setGoal("");
    setCwd("");
    setAdvancedOpen(false);
    setCrewPickerOpen(false);
    setCrewRunners([]);
    void api.crew
      .list()
      .then((rows) => {
        setCrews(rows);
        const launchable = rows.find((c) => c.runner_count > 0);
        const preferred =
          (initialCrewId && rows.find((c) => c.id === initialCrewId)) ||
          launchable ||
          rows[0];
        setCrewId(preferred?.id ?? "");
      })
      .catch((e) => setError(String(e)));
  }, [open, initialCrewId]);

  useEffect(() => {
    if (!open || !crewId) {
      setCrewRunners([]);
      return;
    }
    let cancelled = false;
    void api.crew
      .listRunners(crewId)
      .then((rows) => {
        if (!cancelled) setCrewRunners(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [crewId, open]);

  useEffect(() => {
    if (!crewPickerOpen) return;
    const onPointerDown = (e: MouseEvent) => {
      if (!crewPickerRef.current?.contains(e.target as Node)) {
        setCrewPickerOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCrewPickerOpen(false);
    };
    window.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [crewPickerOpen]);

  const selectedCrew = useMemo(
    () => crews.find((c) => c.id === crewId) ?? null,
    [crews, crewId],
  );

  const launchable = (selectedCrew?.runner_count ?? 0) > 0;
  const lead = crewRunners.find((r) => r.lead) ?? null;
  const workers = crewRunners.filter((r) => !r.lead);
  const selectedCrewSummary = summarizeCrew(selectedCrew, lead, workers);

  const browseCwd = async () => {
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Pick a working directory",
      });
      if (typeof picked === "string") setCwd(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const start = async () => {
    if (!crewId || !title.trim()) return;
    setSubmitting(true);
    setError(null);
    try {
      const out = await api.mission.start({
        crew_id: crewId,
        title: title.trim(),
        goal_override: goal.trim() ? goal.trim() : null,
        cwd: cwd.trim() ? cwd.trim() : null,
      });
      onStarted(out.mission);
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const sessionsHint = selectedCrew
    ? `${selectedCrew.runner_count} session${
        selectedCrew.runner_count === 1 ? "" : "s"
      } will spawn`
    : "";

  return (
    <Modal
      open={open}
      onClose={submitting ? () => {} : onClose}
      title={
        <div className="flex items-center justify-between gap-4">
          <div className="flex flex-col gap-0.5">
            <span className="text-base font-semibold text-fg">
              Start mission
            </span>
            <span className="text-xs font-normal text-fg-2">
              Spawns a session per slot and opens the mission workspace.
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-50"
            aria-label="Close start mission"
          >
            <X aria-hidden className="h-3.5 w-3.5" />
          </button>
        </div>
      }
      widthClass="w-full max-w-[680px]"
      footer={
        <>
          <span className="mr-auto text-[11px] text-fg-3">{sessionsHint}</span>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => void start()}
            disabled={
              submitting || !crewId || !title.trim() || !launchable
            }
          >
            {submitting ? "Starting…" : "Start mission"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-5">
        {error ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
            {error}
          </div>
        ) : null}

        <Field label="Crew">
          <div ref={crewPickerRef} className="relative">
            <button
              type="button"
              disabled={submitting || crews.length === 0}
              onClick={() => setCrewPickerOpen((v) => !v)}
              className="flex w-full cursor-pointer items-center gap-3 rounded-md border border-line bg-bg px-3 py-2.5 text-left transition-colors hover:border-line-strong focus:border-fg-3 focus:outline-none disabled:cursor-default disabled:opacity-60"
              aria-haspopup="listbox"
              aria-expanded={crewPickerOpen}
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] font-semibold text-fg">
                  {selectedCrew?.name ?? "No crews yet"}
                </span>
                <span className="block truncate text-[11px] text-fg-2">
                  {selectedCrewSummary}
                </span>
              </span>
              <ChevronDown aria-hidden className="h-3.5 w-3.5 text-fg-3" />
            </button>
            {crewPickerOpen ? (
              <div
                role="listbox"
                className="absolute left-0 right-0 top-full z-30 mt-1 max-h-56 overflow-y-auto rounded-md border border-line bg-panel p-1 shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
              >
                {crews.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    role="option"
                    aria-selected={c.id === crewId}
                    onClick={() => {
                      setCrewId(c.id);
                      setCrewPickerOpen(false);
                    }}
                    className={`flex w-full cursor-pointer items-center justify-between gap-3 rounded px-2.5 py-2 text-left transition-colors hover:bg-raised ${
                      c.id === crewId ? "bg-raised" : ""
                    }`}
                  >
                    <span className="min-w-0">
                      <span className="block truncate text-[13px] font-semibold text-fg">
                        {c.name}
                      </span>
                      <span className="block truncate text-[11px] text-fg-2">
                        {c.runner_count} runner
                        {c.runner_count === 1 ? "" : "s"}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            ) : null}
          </div>
          {selectedCrew && !launchable ? (
            <p className="mt-1 text-[11px] text-warn">
              This crew has no runners. Add at least one before starting a
              mission.
            </p>
          ) : null}
        </Field>

        <Field
          label="Mission title"
          subtitle="Short label shown in the missions list and event log."
        >
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="e.g. Wire up event bus watcher"
            disabled={submitting}
            className="rounded-md border border-line bg-bg px-3 py-2 text-[13px] text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>

        <Field
          label="Goal"
          subtitle={
            lead
              ? `Delivered to @${lead.handle} (lead) on mission start.`
              : "Delivered to the crew lead on mission start."
          }
        >
          <textarea
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            placeholder={selectedCrew?.goal ?? "Describe what to do…"}
            rows={4}
            disabled={submitting}
            className="min-h-[120px] resize-y rounded-md border border-line bg-bg px-3 py-2 font-mono text-[13px] leading-relaxed text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>

        <Field label="Working directory">
          <div className="flex items-center gap-2">
            <input
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              placeholder="/Users/you/projects/foo (optional)"
              disabled={submitting}
              className="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
            />
            <Button onClick={() => void browseCwd()} disabled={submitting}>
              Browse…
            </Button>
          </div>
          <p className="text-[11px] text-fg-2">
            Each runner&apos;s PTY starts in this directory. Exposed as
            $MISSION_CWD.
          </p>
        </Field>

        <div className="rounded-md border border-line bg-bg px-3.5 py-3">
          <button
            type="button"
            onClick={() => setAdvancedOpen((v) => !v)}
            className="flex w-full cursor-pointer items-center gap-2 text-left text-xs font-medium text-fg hover:text-fg"
          >
            <ChevronRight
              aria-hidden
              className={`h-3.5 w-3.5 text-fg-2 transition-transform ${
                advancedOpen ? "rotate-90" : ""
              }`}
            />
            <span className="min-w-0 flex-1">Advanced</span>
            <span className="text-[11px] font-normal text-fg-3">
              env overrides · per-runner args · attach files
            </span>
          </button>
          {advancedOpen ? (
            <div className="mt-3 rounded border border-line bg-panel px-3 py-2 text-[11px] text-fg-3">
              Reserved for v0.x — per-mission signal-type allowlist
              overrides, custom env, dry-run mode. Inert in v0 MVP.
            </div>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}

function Field({
  label,
  subtitle,
  children,
}: {
  label: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-semibold text-fg">
        {label}
      </span>
      {children}
      {subtitle ? (
        <span className="text-[11px] text-fg-3">{subtitle}</span>
      ) : null}
    </label>
  );
}

function summarizeCrew(
  crew: CrewListItem | null,
  lead: CrewRunner | null,
  workers: CrewRunner[],
): string {
  if (!crew) return "Create a crew first.";
  if (crew.runner_count === 0) return "No runners in this crew.";
  if (!lead) {
    return `${crew.runner_count} runner${
      crew.runner_count === 1 ? "" : "s"
    }`;
  }
  if (workers.length === 0) return `lead: @${lead.handle}`;
  const shownWorkers = workers.slice(0, 3).map((r) => `@${r.handle}`);
  const tail = workers.length > 3 ? `, +${workers.length - 3}` : "";
  return `lead: @${lead.handle} · ${workers.length} worker${
    workers.length === 1 ? "" : "s"
  }: ${shownWorkers.join(", ")}${tail}`;
}
