// Keyboard shortcuts pane — feature #257 v1. Read-only view over the
// static registry in `src/lib/keymap.ts`; rebinding is a designed
// follow-up (see impl 0025 decision 6), so rows render resting-state
// only: title + description left, mono key chips right.

import { useMemo, useState } from "react";
import { Search } from "lucide-react";

import { KEYMAP } from "../../lib/keymap";
import { PaneHeader, SettingsCard } from "./shared";

export function ShortcutsPane() {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return KEYMAP;
    return KEYMAP.filter(
      (entry) =>
        entry.title.toLowerCase().includes(q) ||
        entry.description.toLowerCase().includes(q) ||
        entry.keys.some((key) => key.toLowerCase().includes(q)),
    );
  }, [query]);
  return (
    <>
      <PaneHeader
        title="Keyboard shortcuts"
        subtitle="Bindings are fixed in this version — customization is planned."
      />
      <div className="flex h-9 w-[280px] items-center gap-2 rounded-md border border-line bg-panel px-2.5">
        <Search aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
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
            <div
              key={entry.id}
              className="flex items-center justify-between gap-6 px-4 py-3"
            >
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="text-[13px] font-medium text-fg">
                  {entry.title}
                </span>
                <span className="text-[11px] text-fg-2">
                  {entry.description}
                </span>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                {entry.keys.map((key) => (
                  <kbd
                    key={key}
                    className="rounded border border-line bg-raised px-1.5 py-0.5 font-mono text-[11px] leading-tight text-fg-2"
                  >
                    {key}
                  </kbd>
                ))}
              </div>
            </div>
          ))}
        </SettingsCard>
      )}
    </>
  );
}
