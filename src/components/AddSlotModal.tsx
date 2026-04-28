// Add an existing runner to a crew slot (C5.5 shared-runner model).
//
// Runner creation lives on the top-level Runners page. This modal mirrors
// design/runners-design.pen node `sYprG`: search existing runners, select
// one, then add it to the crew. Per-slot prompt overrides are shown as the
// v0.x placeholder from the design because the schema has no override
// column yet.

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ChevronDown, X } from "lucide-react";

import { api } from "../lib/api";
import type { RunnerWithActivity } from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";

export function AddSlotModal({
  open,
  crewId,
  crewName,
  currentRunnerIds,
  onClose,
  onCreated,
}: {
  open: boolean;
  crewId: string;
  crewName: string;
  currentRunnerIds: string[];
  onClose: () => void;
  onCreated: () => void | Promise<void>;
}) {
  const [runners, setRunners] = useState<RunnerWithActivity[]>([]);
  const [query, setQuery] = useState("");
  const [selectedRunnerId, setSelectedRunnerId] = useState("");
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setQuery("");
    setSelectedRunnerId("");
    setError(null);
    setSubmitting(false);
    setLoading(true);
    void api.runner
      .listWithActivity()
      .then((rows) => {
        if (cancelled) return;
        setRunners(rows);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const currentIds = useMemo(
    () => new Set(currentRunnerIds),
    [currentRunnerIds],
  );

  const availableRunners = useMemo(() => {
    const q = query.trim().toLowerCase();
    return runners
      .filter((r) => !currentIds.has(r.id))
      .filter((r) => {
        if (!q) return true;
        return (
          r.handle.toLowerCase().includes(q) ||
          r.display_name.toLowerCase().includes(q) ||
          r.role.toLowerCase().includes(q) ||
          r.runtime.toLowerCase().includes(q)
        );
      });
  }, [currentIds, query, runners]);

  useEffect(() => {
    if (!open) return;
    setSelectedRunnerId((id) => {
      if (id && availableRunners.some((r) => r.id === id)) return id;
      return availableRunners[0]?.id ?? "";
    });
  }, [availableRunners, open]);

  const selectedRunner =
    availableRunners.find((r) => r.id === selectedRunnerId) ?? null;
  const canSubmit = selectedRunner !== null && !submitting && !loading;

  const submit = async () => {
    if (!selectedRunner) return;
    setSubmitting(true);
    setError(null);
    try {
      await api.crew.addRunner(crewId, selectedRunner.id);
      await onCreated();
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  const openCreateRunner = () => {
    onClose();
    navigate("/runners", { state: { createRunner: true } });
  };

  return (
    <Modal
      open={open}
      onClose={submitting ? () => {} : onClose}
      title={
        <div className="flex items-center justify-between gap-4">
          <div className="flex flex-col gap-0.5">
            <span className="text-base font-semibold text-fg">Add slot</span>
            <span className="text-xs font-normal text-fg-2">
              crew: {crewName}
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            className="inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg disabled:pointer-events-none disabled:opacity-50"
            aria-label="Close add slot"
          >
            <X aria-hidden className="h-3.5 w-3.5" />
          </button>
        </div>
      }
      widthClass="w-full max-w-xl"
      footer={
        <>
          <Button onClick={onClose} disabled={submitting}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={!canSubmit}>
            {submitting ? "Adding..." : "Add slot"}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <section className="flex flex-col gap-1.5">
          <label
            htmlFor="add-slot-runner-search"
            className="text-xs font-semibold text-fg"
          >
            Runner
          </label>
          <div className="flex items-center gap-2 rounded-md border border-line bg-bg px-3 py-2 text-[13px] focus-within:border-fg-3">
            <input
              id="add-slot-runner-search"
              autoFocus
              value={query}
              placeholder="Search runners..."
              onChange={(e) => setQuery(e.target.value)}
              className="min-w-0 flex-1 bg-transparent text-fg outline-none placeholder:text-fg-3"
            />
            <ChevronDown aria-hidden className="h-3.5 w-3.5 text-fg-3" />
          </div>

          <div className="overflow-hidden rounded-md border border-line bg-panel">
            <button
              type="button"
              onClick={openCreateRunner}
              className="flex w-full cursor-pointer items-center border-b border-line px-3 py-2.5 text-left text-[13px] font-medium text-accent transition-colors hover:bg-raised"
            >
              + Create new runner...
            </button>

            {loading ? (
              <div className="px-3 py-3 text-xs text-fg-3">
                Loading runners...
              </div>
            ) : availableRunners.length === 0 ? (
              <div className="px-3 py-3 text-xs text-fg-3">
                {runners.length === 0
                  ? "No runners yet. Create one first, then add it here."
                  : query.trim()
                    ? "No available runners match this search."
                    : "Every runner is already in this crew."}
              </div>
            ) : (
              <div className="max-h-56 overflow-y-auto">
                {availableRunners.map((runner) => (
                  <RunnerOption
                    key={runner.id}
                    runner={runner}
                    selected={runner.id === selectedRunnerId}
                    onSelect={() => setSelectedRunnerId(runner.id)}
                  />
                ))}
              </div>
            )}
          </div>
        </section>

        <section className="flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs font-semibold text-fg">
              System prompt override
            </span>
            <span
              aria-label="System prompt override off"
              className="flex h-[18px] w-8 items-center rounded-full bg-raised p-0.5"
              title="Per-slot prompt overrides land in v0.x"
            >
              <span className="h-3.5 w-3.5 rounded-full bg-panel" />
            </span>
          </div>
          <p className="text-[11px] text-fg-2">
            Off — uses the runner&apos;s default prompt. Turn on to override
            for this slot only.
          </p>
        </section>

        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </form>
    </Modal>
  );
}

function RunnerOption({
  runner,
  selected,
  onSelect,
}: {
  runner: RunnerWithActivity;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`grid w-full cursor-pointer grid-cols-[minmax(0,160px)_80px_minmax(0,1fr)] items-center gap-3 border-b border-line px-3 py-2.5 text-left last:border-b-0 transition-colors hover:bg-raised ${
        selected ? "bg-raised" : ""
      }`}
    >
      <span className="truncate font-mono text-[13px] font-semibold text-accent">
        @{runner.handle}
      </span>
      <span className="truncate text-[11px] text-fg-2">{runner.runtime}</span>
      <span className="truncate text-xs text-fg-2">
        {crewUsageLabel(runner)} · {activityLabel(runner)}
      </span>
    </button>
  );
}

function crewUsageLabel(runner: RunnerWithActivity): string {
  return runner.crew_count === 1
    ? "in 1 crew"
    : `in ${runner.crew_count} crews`;
}

function activityLabel(runner: RunnerWithActivity): string {
  if (runner.active_sessions > 0) {
    return runner.active_sessions === 1
      ? "1 session"
      : `${runner.active_sessions} sessions`;
  }
  if (runner.active_missions > 0) {
    return runner.active_missions === 1
      ? "1 mission"
      : `${runner.active_missions} missions`;
  }
  return "idle";
}
