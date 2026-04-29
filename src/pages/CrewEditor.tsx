// Crew detail — matches design/runners-design.pen frame `CUKjM`.
//
// Layout: top toolbar (back to Crews + inline name field + Save + Start
// mission) above a two-section body (Purpose, Slots).

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
} from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { MoreHorizontal, SquarePen, Star, Trash2 } from "lucide-react";

import { api } from "../lib/api";
import type { Crew, Runner, SlotWithRunner } from "../lib/types";
import { AddSlotModal } from "../components/AddSlotModal";
import { RunnerEditDrawer } from "../components/RunnerEditDrawer";
import { StartMissionModal } from "../components/StartMissionModal";
import { Button } from "../components/ui/Button";

export default function CrewEditor() {
  const { crewId } = useParams<{ crewId: string }>();
  const [crew, setCrew] = useState<Crew | null>(null);
  const [slots, setSlots] = useState<SlotWithRunner[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [starting, setStarting] = useState(false);
  const [editing, setEditing] = useState<Runner | null>(null);
  const [nameDraft, setNameDraft] = useState("");
  const [savingName, setSavingName] = useState(false);
  const [reordering, setReordering] = useState(false);
  const reorderInFlight = useRef(false);
  const navigate = useNavigate();

  const refresh = useCallback(async () => {
    if (!crewId) return;
    try {
      setError(null);
      const [c, rs] = await Promise.all([
        api.crew.get(crewId),
        api.slot.list(crewId),
      ]);
      setCrew(c);
      setSlots(rs);
      setNameDraft(c.name);
      setLoaded(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [crewId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onSaveName = async () => {
    if (!crew || !crewId) return;
    const next = nameDraft.trim();
    if (!next || next === crew.name) return;
    setSavingName(true);
    try {
      await api.crew.update(crewId, { name: next });
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingName(false);
    }
  };

  const onSetLead = async (slotId: string) => {
    try {
      await api.slot.setLead(slotId);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const onRemoveSlot = async (s: SlotWithRunner) => {
    const tail = s.lead
      ? "\nAs the LEAD, leadership will pass to the next slot by position."
      : "";
    if (!confirm(`Remove slot @${s.slot_handle} from this crew?${tail}`)) return;
    try {
      await api.slot.delete(s.id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const existingSlotHandles = useMemo(
    () => slots.map((s) => s.slot_handle),
    [slots],
  );

  const onCommitReorder = async (newOrder: SlotWithRunner[]) => {
    if (!crewId) return;
    if (reorderInFlight.current) return;
    reorderInFlight.current = true;
    setReordering(true);
    setSlots(newOrder);
    try {
      const updated = await api.slot.reorder(
        crewId,
        newOrder.map((s) => s.id),
      );
      setSlots(updated);
    } catch (e) {
      setError(String(e));
      await refresh();
    } finally {
      reorderInFlight.current = false;
      setReordering(false);
    }
  };

  if (!crewId) {
    return <div className="p-8 text-sm text-danger">Missing crew id.</div>;
  }

  const trimmedNameDraft = nameDraft.trim();
  const nameChanged = crew !== null && nameDraft !== crew.name;
  const nameInvalid = crew !== null && trimmedNameDraft.length === 0;
  const nameDirty =
    crew !== null && trimmedNameDraft !== crew.name && !nameInvalid;

  return (
    <>
      <div className="flex items-center justify-between gap-4 border-b border-line bg-panel px-8 pb-4 pt-9">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <Link
            to="/crews"
            className="shrink-0 text-sm text-fg-2 transition-colors hover:text-fg"
          >
            ‹ Crews
          </Link>
          <span className="text-line-strong">›</span>
          {crew ? (
            <input
              value={nameDraft}
              onChange={(e) => setNameDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onSaveName();
                }
                if (e.key === "Escape") {
                  setNameDraft(crew.name);
                  (e.target as HTMLInputElement).blur();
                }
              }}
              className="min-w-0 max-w-sm rounded border border-line bg-bg px-2.5 py-1.5 text-sm font-semibold text-fg focus:border-fg-3 focus:outline-none"
            />
          ) : (
            <span className="text-sm text-fg-3">…</span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {nameChanged || savingName ? (
            <Button
              onClick={onSaveName}
              disabled={savingName || !nameDirty}
              title={
                nameInvalid
                  ? "Crew name cannot be empty"
                  : nameDirty
                    ? "Save crew name"
                    : "No persisted change after trimming"
              }
            >
              {savingName ? "Saving..." : "Save"}
            </Button>
          ) : (
            <span
              className="inline-flex items-center justify-center rounded border border-line bg-raised px-3 py-1.5 text-sm font-medium text-fg-3"
              title="Crew name is saved. Slot changes save immediately."
            >
              Saved
            </span>
          )}
          <Button
            variant="primary"
            onClick={() => setStarting(true)}
            disabled={slots.length === 0}
            title={
              slots.length === 0
                ? "Add at least one slot before starting a mission"
                : "Start a mission with this crew"
            }
          >
            Start mission
          </Button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="p-8 text-sm text-fg-2">Loading…</div>
        ) : !loaded ? (
          <div className="m-8 rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
            {error ?? "Failed to load crew."}
          </div>
        ) : crew === null ? (
          <div className="p-8 text-sm text-danger">Crew not found.</div>
        ) : (
          <div className="mx-auto flex max-w-4xl flex-col gap-8 px-8 py-8">
            {error ? (
              <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
                {error}
              </div>
            ) : null}

            <section className="flex flex-col gap-1.5">
              <div className="text-[10px] font-semibold uppercase tracking-[0.15em] text-fg-3">
                Purpose
              </div>
              {crew.purpose ? (
                <p className="text-sm text-fg">{crew.purpose}</p>
              ) : (
                <p className="text-sm italic text-fg-3">No purpose set.</p>
              )}
            </section>

            <section className="flex flex-col gap-4">
              <div className="flex items-end justify-between gap-4">
                <div className="flex flex-col gap-0.5">
                  <h2 className="text-xl font-bold text-fg">Slots</h2>
                  <p className="text-xs text-fg-2">
                    Positions in the crew. Each slot binds a handle to a runner.
                    The{" "}
                    <span className="font-semibold text-accent">LEAD</span> is the
                    crew's face — receives human messages by default and dispatches
                    back to other slots.
                  </p>
                </div>
                <Button
                  variant="primary"
                  className="shrink-0 whitespace-nowrap"
                  onClick={() => setAdding(true)}
                >
                  + Add slot
                </Button>
              </div>

              <SlotList
                slots={slots}
                reordering={reordering}
                onSetLead={onSetLead}
                onEdit={(s) => setEditing(s.runner)}
                onRemove={onRemoveSlot}
                onReorder={onCommitReorder}
              />
            </section>
          </div>
        )}
      </div>

      <AddSlotModal
        open={adding}
        crewId={crewId}
        crewName={crew?.name ?? nameDraft}
        existingSlotHandles={existingSlotHandles}
        onClose={() => setAdding(false)}
        onCreated={async () => {
          setAdding(false);
          await refresh();
        }}
      />

      <RunnerEditDrawer
        open={editing !== null}
        runner={editing}
        onClose={() => setEditing(null)}
        onSaved={async () => {
          setEditing(null);
          await refresh();
        }}
      />

      <StartMissionModal
        open={starting}
        initialCrewId={crewId}
        onClose={() => setStarting(false)}
        onStarted={(mission) => {
          setStarting(false);
          navigate(`/missions/${mission.id}`);
        }}
      />
    </>
  );
}

function SlotList({
  slots,
  reordering,
  onSetLead,
  onEdit,
  onRemove,
  onReorder,
}: {
  slots: SlotWithRunner[];
  reordering: boolean;
  onSetLead: (slotId: string) => void;
  onEdit: (s: SlotWithRunner) => void;
  onRemove: (s: SlotWithRunner) => void;
  onReorder: (newOrder: SlotWithRunner[]) => void;
}) {
  if (slots.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-line-strong bg-panel/40 px-5 py-8 text-center">
        <p className="text-sm text-fg">No slots yet.</p>
        <p className="mt-1 text-xs text-fg-3">
          Use <span className="font-medium text-fg">+ Add slot</span> above —
          the first slot auto-assigns as LEAD.
        </p>
      </div>
    );
  }
  return (
    <ol className="flex flex-col gap-2">
      {slots.map((s, i) => (
        <SlotRow
          key={s.id}
          slot={s}
          index={i}
          total={slots.length}
          dragDisabled={reordering}
          onSetLead={() => onSetLead(s.id)}
          onEdit={() => onEdit(s)}
          onRemove={() => onRemove(s)}
          onReorderDrop={(fromIndex) => {
            if (fromIndex === i) return;
            const next = moveItem(slots, fromIndex, i);
            onReorder(next);
          }}
        />
      ))}
    </ol>
  );
}

function moveItem<T>(arr: T[], from: number, to: number): T[] {
  const copy = arr.slice();
  const [item] = copy.splice(from, 1);
  copy.splice(to, 0, item);
  return copy;
}

function SlotRow({
  slot,
  index,
  total,
  dragDisabled,
  onSetLead,
  onEdit,
  onRemove,
  onReorderDrop,
}: {
  slot: SlotWithRunner;
  index: number;
  total: number;
  dragDisabled: boolean;
  onSetLead: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onReorderDrop: (fromIndex: number) => void;
}) {
  const [dragOver, setDragOver] = useState(false);
  const draggable = total > 1 && !dragDisabled;

  const onDragStart = (e: DragEvent<HTMLLIElement>) => {
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(index));
  };
  const onDragOver = (e: DragEvent<HTMLLIElement>) => {
    if (dragDisabled) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setDragOver(true);
  };
  const onDragLeave = () => setDragOver(false);
  const onDrop = (e: DragEvent<HTMLLIElement>) => {
    if (dragDisabled) return;
    e.preventDefault();
    setDragOver(false);
    const from = Number(e.dataTransfer.getData("text/plain"));
    if (!Number.isNaN(from)) onReorderDrop(from);
  };

  const runner = slot.runner;
  const summary = useMemo(() => {
    const parts = [runner.command, ...runner.args];
    return parts.filter(Boolean).join(" ");
  }, [runner.command, runner.args]);

  return (
    <li
      draggable={draggable}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`group flex items-center gap-4 rounded-lg border bg-panel p-4 transition-colors ${
        dragOver
          ? "border-accent/50 bg-accent/5"
          : "border-line hover:border-line-strong"
      }`}
    >
      <div
        className={`flex shrink-0 select-none items-center text-[14px] leading-none text-fg-3 ${
          draggable ? "cursor-grab" : "opacity-40"
        }`}
        title={draggable ? "Drag to reorder" : undefined}
      >
        ⋮⋮
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-mono text-[13px] font-medium text-fg">
            @{slot.slot_handle}
          </span>
          {slot.lead ? (
            <span className="rounded bg-accent/10 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-accent">
              Lead
            </span>
          ) : null}
          <span className="rounded bg-raised px-1.5 py-0.5 text-[10px] font-medium text-fg-2">
            {runner.runtime}
          </span>
          <span className="font-mono text-[11px] text-fg-3">
            from @{runner.handle}
          </span>
        </div>
        {runner.system_prompt ? (
          <div className="mt-1 line-clamp-1 text-xs text-fg-2">
            {runner.system_prompt}
          </div>
        ) : null}
        {summary ? (
          <div className="mt-1 truncate font-mono text-[11px] text-fg-3">
            $ {summary}
          </div>
        ) : null}
      </div>

      <SlotActionMenu
        slot={slot}
        onSetLead={onSetLead}
        onEdit={onEdit}
        onRemove={onRemove}
      />
    </li>
  );
}

function SlotActionMenu({
  slot,
  onSetLead,
  onEdit,
  onRemove,
}: {
  slot: SlotWithRunner;
  onSetLead: () => void;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div
      ref={menuRef}
      className="relative shrink-0 opacity-70 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100"
    >
      <button
        type="button"
        aria-label={`Slot actions for @${slot.slot_handle}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
        onMouseDown={(e) => e.stopPropagation()}
        className="inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded text-fg-2 transition-colors hover:bg-raised hover:text-fg focus:outline-none focus-visible:bg-raised focus-visible:text-fg"
        title="Slot actions"
      >
        <MoreHorizontal aria-hidden className="h-4 w-4" />
      </button>

      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-30 mt-2 flex w-52 flex-col gap-px rounded-lg border border-line bg-panel p-1.5 text-[13px] shadow-[0_8px_30px_rgba(0,0,0,0.67)]"
          onClick={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            disabled={slot.lead}
            onClick={() => {
              if (slot.lead) return;
              setOpen(false);
              onSetLead();
            }}
            className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-fg transition-colors hover:bg-raised disabled:cursor-default disabled:text-fg-3 disabled:hover:bg-transparent"
          >
            <Star
              aria-hidden
              className={`h-3.5 w-3.5 ${
                slot.lead ? "text-fg-3" : "text-warn"
              }`}
            />
            <span>{slot.lead ? "Current lead" : "Set as lead"}</span>
          </button>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onEdit();
            }}
            className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-fg transition-colors hover:bg-raised"
          >
            <SquarePen aria-hidden className="h-3.5 w-3.5 text-fg" />
            <span>Edit runner</span>
          </button>

          <div className="px-1 py-1">
            <div className="h-px bg-line" />
          </div>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onRemove();
            }}
            className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-danger transition-colors hover:bg-raised"
          >
            <Trash2 aria-hidden className="h-3.5 w-3.5" />
            <span>Remove from crew</span>
          </button>
        </div>
      ) : null}
    </div>
  );
}
