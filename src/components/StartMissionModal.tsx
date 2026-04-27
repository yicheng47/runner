// Start Mission modal (C11) — entrypoint for everything C10 renders.
//
// Crew picker is filtered to crews that look launchable (≥1 runner). The
// validation that actually rejects the mission lives on the Rust side:
// `mission_start` checks for runners + a lead and returns a clean Error
// either way; the modal surfaces that error inline so an empty-crew
// pick is recoverable without closing.

import { useEffect, useMemo, useState } from "react";

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { api } from "../lib/api";
import type { CrewListItem, Mission } from "../lib/types";
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
  const [title, setTitle] = useState("");
  const [goal, setGoal] = useState("");
  const [cwd, setCwd] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const selectedCrew = useMemo(
    () => crews.find((c) => c.id === crewId) ?? null,
    [crews, crewId],
  );

  const launchable = (selectedCrew?.runner_count ?? 0) > 0;

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
      onClose={onClose}
      title="Start mission"
      widthClass="w-full max-w-xl"
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
      <div className="flex flex-col gap-4">
        {error ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger">
            {error}
          </div>
        ) : null}

        <Field label="Crew">
          <select
            value={crewId}
            onChange={(e) => setCrewId(e.target.value)}
            disabled={submitting}
            className="rounded border border-line bg-bg px-2 py-1.5 text-sm text-fg focus:border-fg-3 focus:outline-none"
          >
            {crews.length === 0 ? (
              <option value="">— no crews yet —</option>
            ) : (
              crews.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} · {c.runner_count} runner
                  {c.runner_count === 1 ? "" : "s"}
                </option>
              ))
            )}
          </select>
          {selectedCrew && !launchable ? (
            <p className="mt-1 text-[11px] text-warn">
              This crew has no runners. Add at least one before starting a
              mission.
            </p>
          ) : null}
        </Field>

        <Field label="Title">
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="e.g. Wire up event bus watcher"
            disabled={submitting}
            className="rounded border border-line bg-bg px-2 py-1.5 text-sm text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>

        <Field
          label="Goal"
          subtitle={
            selectedCrew?.goal
              ? `Defaults to crew goal: "${selectedCrew.goal}"`
              : "Optional override of the crew's default goal."
          }
        >
          <textarea
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            placeholder={selectedCrew?.goal ?? "Describe what to do…"}
            rows={4}
            disabled={submitting}
            className="resize-y rounded border border-line bg-bg px-2 py-1.5 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
          />
        </Field>

        <Field label="Working directory">
          <div className="flex items-center gap-2">
            <input
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              placeholder="/Users/you/projects/foo (optional)"
              disabled={submitting}
              className="flex-1 rounded border border-line bg-bg px-2 py-1.5 font-mono text-xs text-fg placeholder:text-fg-3 focus:border-fg-3 focus:outline-none"
            />
            <Button onClick={() => void browseCwd()} disabled={submitting}>
              Browse…
            </Button>
          </div>
        </Field>

        <div>
          <button
            type="button"
            onClick={() => setAdvancedOpen((v) => !v)}
            className="cursor-pointer text-[11px] font-medium text-fg-2 hover:text-fg"
          >
            {advancedOpen ? "▾" : "▸"} Advanced
          </button>
          {advancedOpen ? (
            <div className="mt-2 rounded border border-line bg-bg px-3 py-2 text-[11px] text-fg-3">
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
    <label className="flex flex-col gap-1">
      <span className="text-[11px] font-medium uppercase tracking-wide text-fg-3">
        {label}
      </span>
      {children}
      {subtitle ? (
        <span className="text-[11px] text-fg-3">{subtitle}</span>
      ) : null}
    </label>
  );
}
