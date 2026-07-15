import { useEffect, useState } from "react";
import { Pencil, RotateCcw, Search, Trash2 } from "lucide-react";

import {
  KEYMAP,
  KEYMAP_CHANGED_EVENT,
  clearKeymapOverride,
  comboFromEvent,
  effectiveBinding,
  findKeymapConflict,
  formatCombo,
  readKeymapOverrides,
  resetKeymapOverrides,
  setKeymapOverride,
  suspendShortcutMatching,
  type KeymapEntry,
} from "../../lib/keymap";
import { PaneHeader, SettingsCard } from "./shared";

const STORAGE_KEYMAP_OVERRIDES = "settings.keymapOverrides";

interface ConflictNotice {
  id: string;
  message: string;
}

export function ShortcutsPane() {
  const [query, setQuery] = useState("");
  const [recordingId, setRecordingId] = useState<string | null>(null);
  const [conflict, setConflict] = useState<ConflictNotice | null>(null);
  const [, setKeymapVersion] = useState(0);

  useEffect(() => {
    const refresh = () => {
      setKeymapVersion((version) => version + 1);
      setConflict(null);
    };
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === STORAGE_KEYMAP_OVERRIDES) {
        refresh();
      }
    };
    window.addEventListener(KEYMAP_CHANGED_EVENT, refresh);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(KEYMAP_CHANGED_EVENT, refresh);
      window.removeEventListener("storage", onStorage);
    };
  }, []);

  useEffect(() => {
    if (!recordingId) return;
    suspendShortcutMatching(true);
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        setRecordingId(null);
        setConflict(null);
        return;
      }
      const combo = comboFromEvent(event);
      if (!combo) return;
      const existing = findKeymapConflict(combo, recordingId);
      if (existing) {
        setConflict({
          id: recordingId,
          message: `Already used by ${existing.title}`,
        });
        return;
      }
      setKeymapOverride(recordingId, combo);
      setRecordingId(null);
      setConflict(null);
    };
    const onPointerDown = (event: PointerEvent) => {
      if (
        event.target instanceof Element &&
        event.target.closest("[data-shortcut-recorder]")
      ) {
        return;
      }
      setRecordingId(null);
      setConflict(null);
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    window.addEventListener("pointerdown", onPointerDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKeyDown, { capture: true });
      window.removeEventListener("pointerdown", onPointerDown, {
        capture: true,
      });
      suspendShortcutMatching(false);
    };
  }, [recordingId]);

  const overrides = readKeymapOverrides();
  const bindings = new Map(
    KEYMAP.map((entry) => [entry.id, effectiveBinding(entry.id)]),
  );
  const q = query.trim().toLowerCase();
  const filtered = q
    ? KEYMAP.filter((entry) => {
        const binding = bindings.get(entry.id);
        return (
          entry.title.toLowerCase().includes(q) ||
          entry.description.toLowerCase().includes(q) ||
          (binding ? formatCombo(binding) : "unassigned")
            .toLowerCase()
            .includes(q)
        );
      })
    : KEYMAP;
  const hasOverrides = Object.keys(overrides).length > 0;

  const startRecording = (id: string) => {
    setRecordingId(id);
    setConflict(null);
  };

  const resetAll = () => {
    setRecordingId(null);
    setConflict(null);
    resetKeymapOverrides();
  };

  const restoreDefault = (id: string) => {
    const existing = clearKeymapOverride(id);
    if (existing) {
      setConflict({ id, message: `Already used by ${existing.title}` });
    }
  };

  return (
    <>
      <PaneHeader
        title="Keyboard shortcuts"
        subtitle="Shortcuts must include ⌘, Control, or Option. Function keys can be used alone."
        action={
          <button
            type="button"
            onClick={resetAll}
            disabled={!hasOverrides}
            className="flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-line bg-raised px-3 py-1.5 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg disabled:cursor-default disabled:opacity-40 disabled:hover:border-line disabled:hover:text-fg-2"
          >
            <RotateCcw aria-hidden className="h-3 w-3" />
            Reset all to defaults
          </button>
        }
      />
      <div className="flex h-9 w-[280px] items-center gap-2 rounded-md border border-line bg-panel px-2.5">
        <Search aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search shortcuts"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-fg-3"
        />
      </div>
      {filtered.length === 0 ? (
        <p className="text-[12px] text-fg-3">
          No shortcuts match “{query.trim()}”.
        </p>
      ) : (
        <SettingsCard>
          {filtered.map((entry) => (
            <ShortcutRow
              key={entry.id}
              entry={entry}
              binding={bindings.get(entry.id) ?? null}
              overridden={entry.id in overrides}
              recording={recordingId === entry.id}
              conflict={conflict?.id === entry.id ? conflict.message : null}
              onRecord={() => startRecording(entry.id)}
              onUnbind={() => setKeymapOverride(entry.id, null)}
              onRestore={() => restoreDefault(entry.id)}
            />
          ))}
        </SettingsCard>
      )}
    </>
  );
}

function ShortcutRow({
  entry,
  binding,
  overridden,
  recording,
  conflict,
  onRecord,
  onUnbind,
  onRestore,
}: {
  entry: KeymapEntry;
  binding: ReturnType<typeof effectiveBinding>;
  overridden: boolean;
  recording: boolean;
  conflict: string | null;
  onRecord: () => void;
  onUnbind: () => void;
  onRestore: () => void;
}) {
  const iconButtonClass =
    "flex h-6 w-6 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg focus-visible:bg-raised focus-visible:text-fg focus-visible:outline-none";
  return (
    <div className="grid min-h-[58px] grid-cols-[minmax(0,1fr)_minmax(0,1fr)_24px] items-center gap-4 px-4 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-fg">{entry.title}</span>
        <span className="text-[11px] text-fg-2">{entry.description}</span>
        {conflict ? (
          <span className="text-[11px] text-danger">{conflict}</span>
        ) : null}
      </div>
      {recording ? (
        <>
          <div
            data-shortcut-recorder
            className="flex h-8 w-[184px] max-w-full items-center justify-center rounded-md border border-line-strong bg-bg px-3 shadow-inner"
          >
            <span className="text-[12px] text-fg-2">Press keys…</span>
          </div>
          <span aria-hidden />
        </>
      ) : (
        <>
          <div className="flex min-w-0 items-center gap-1.5">
            {entry.fixed ? (
              <kbd className="rounded border border-line bg-raised px-2 py-1 font-mono text-[11px] leading-tight text-fg-2">
                {formatCombo(binding ?? entry.default)}
              </kbd>
            ) : (
              <button
                type="button"
                onClick={onRecord}
                aria-label={`Edit ${entry.title} shortcut`}
                className="cursor-pointer rounded border border-line bg-raised px-2 py-1 font-mono text-[11px] leading-tight text-fg-2 transition-colors hover:border-line-strong hover:text-fg focus-visible:border-line-strong focus-visible:text-fg focus-visible:outline-none"
              >
                {binding ? formatCombo(binding) : "Unassigned"}
              </button>
            )}
            {!entry.fixed ? (
              <button
                type="button"
                onClick={onRecord}
                aria-label={`Edit ${entry.title} shortcut`}
                title="Edit shortcut"
                className={iconButtonClass}
              >
                <Pencil aria-hidden className="h-3.5 w-3.5" />
              </button>
            ) : null}
            {overridden ? (
              <button
                type="button"
                onClick={onRestore}
                aria-label={`Restore default for ${entry.title}`}
                title="Restore default"
                className={iconButtonClass}
              >
                <RotateCcw aria-hidden className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
          {!entry.fixed ? (
            <button
              type="button"
              onClick={onUnbind}
              aria-label={`Unbind ${entry.title}`}
              title="Unbind shortcut"
              className={iconButtonClass}
            >
              <Trash2 aria-hidden className="h-3.5 w-3.5" />
            </button>
          ) : (
            <span aria-hidden />
          )}
        </>
      )}
    </div>
  );
}
