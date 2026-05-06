// Add a slot to a crew. A slot binds a runner template to an in-crew
// identity (`slot_handle`). The same runner template may fill
// multiple slots in one crew with different slot_handles — see
// docs/impls/0002-crew-slots.md.

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ChevronDown, X } from "lucide-react";

import { api } from "../lib/api";
import type { RunnerWithActivity } from "../lib/types";
import { Button } from "./ui/Button";
import { Modal } from "./ui/Overlay";
import { Field } from "./ui/Field";

const HANDLE_RE = /^[a-z0-9][a-z0-9_-]{0,31}$/;

export function AddSlotModal({
  open,
  crewId,
  crewName,
  existingSlotHandles,
  onClose,
  onCreated,
}: {
  open: boolean;
  crewId: string;
  crewName: string;
  /** Slot handles already used in this crew. Drives the auto-suggested
   *  default and the inline duplicate warning. */
  existingSlotHandles: string[];
  onClose: () => void;
  onCreated: () => void | Promise<void>;
}) {
  const [runners, setRunners] = useState<RunnerWithActivity[]>([]);
  const [query, setQuery] = useState("");
  const [selectedRunnerId, setSelectedRunnerId] = useState("");
  const [slotHandle, setSlotHandle] = useState("");
  const [slotHandleEdited, setSlotHandleEdited] = useState(false);
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setQuery("");
    setSelectedRunnerId("");
    setSlotHandle("");
    setSlotHandleEdited(false);
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

  const taken = useMemo(
    () => new Set(existingSlotHandles),
    [existingSlotHandles],
  );

  const filteredRunners = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return runners;
    return runners.filter(
      (r) =>
        r.handle.toLowerCase().includes(q) ||
        r.display_name.toLowerCase().includes(q) ||
        r.runtime.toLowerCase().includes(q),
    );
  }, [query, runners]);

  useEffect(() => {
    if (!open) return;
    setSelectedRunnerId((id) => {
      if (id && filteredRunners.some((r) => r.id === id)) return id;
      return filteredRunners[0]?.id ?? "";
    });
  }, [filteredRunners, open]);

  const selectedRunner =
    runners.find((r) => r.id === selectedRunnerId) ?? null;

  // Auto-suggest a slot_handle when the user picks a runner. If the
  // runner's own handle is already taken, append -2/-3 until free.
  useEffect(() => {
    if (slotHandleEdited) return;
    if (!selectedRunner) {
      setSlotHandle("");
      return;
    }
    const base = selectedRunner.handle;
    if (!taken.has(base)) {
      setSlotHandle(base);
      return;
    }
    for (let i = 2; i < 100; i += 1) {
      const candidate = `${base}-${i}`;
      if (!taken.has(candidate)) {
        setSlotHandle(candidate);
        return;
      }
    }
    setSlotHandle(base);
  }, [selectedRunner, slotHandleEdited, taken]);

  const slotHandleError = (() => {
    if (!slotHandle) return null;
    if (!HANDLE_RE.test(slotHandle))
      return "Lowercase letters, digits, '-' or '_'; must start with a letter or digit; up to 32 chars.";
    if (taken.has(slotHandle))
      return `'${slotHandle}' is already used in this crew.`;
    return null;
  })();

  const canSubmit =
    selectedRunner !== null &&
    slotHandle.length > 0 &&
    slotHandleError === null &&
    !submitting &&
    !loading;

  const submit = async () => {
    if (!selectedRunner || !canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      await api.slot.create({
        crew_id: crewId,
        runner_id: selectedRunner.id,
        slot_handle: slotHandle,
      });
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
            ) : filteredRunners.length === 0 ? (
              <div className="px-3 py-3 text-xs text-fg-3">
                {runners.length === 0
                  ? "No runners yet. Create one first, then add it here."
                  : "No runners match this search."}
              </div>
            ) : (
              <div className="max-h-56 overflow-y-auto">
                {filteredRunners.map((runner) => (
                  <RunnerOption
                    key={runner.id}
                    runner={runner}
                    selected={runner.id === selectedRunnerId}
                    onSelect={() => {
                      setSelectedRunnerId(runner.id);
                      setSlotHandleEdited(false);
                    }}
                  />
                ))}
              </div>
            )}
          </div>
        </section>

        <Field
          id="add-slot-handle"
          label="Slot handle"
          hint="in-crew identity used by mission events and stdin routing"
          error={slotHandleError}
        >
          <div className="flex items-center rounded border border-line-strong bg-bg px-2.5 py-1.5 text-sm focus-within:border-fg-3">
            <span className="select-none pr-1 font-mono font-semibold text-fg-3">
              @
            </span>
            <input
              id="add-slot-handle"
              value={slotHandle}
              placeholder="architect"
              onChange={(e) => {
                setSlotHandle(e.target.value.toLowerCase());
                setSlotHandleEdited(true);
              }}
              className="flex-1 bg-transparent font-mono text-fg outline-none placeholder:text-fg-3"
            />
          </div>
        </Field>

        <section className="flex flex-col gap-2 opacity-70">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold text-fg">
                System prompt override
              </span>
              <span className="rounded bg-raised px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-fg-3">
                v0.x
              </span>
            </div>
            <span
              aria-label="System prompt override unavailable"
              className="flex h-[18px] w-8 items-center rounded-full bg-raised p-0.5"
              title="Per-slot prompt overrides land in v0.x"
            >
              <span className="h-3.5 w-3.5 rounded-full bg-panel" />
            </span>
          </div>
          <p className="text-[11px] text-fg-2">
            Uses the selected runner&apos;s default prompt. Per-slot overrides
            are not editable in the MVP.
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
