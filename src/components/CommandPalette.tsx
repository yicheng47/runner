// Command palette — Pencil node `Fkoe8`. Opens from the sidebar's
// search row (and ⌘K / Ctrl+K). Lists missions, runners, and crews
// in one searchable surface; selecting a row navigates to the
// corresponding detail page.
//
// Filter is plain substring match against the visible label
// (mission title / runner handle / crew name) — fast, no scoring,
// no fuzzy. Empty query shows everything sorted by recency
// (missions/runners/crews each ordered by their natural recency
// signal). The arrow keys + Enter move/select; Escape and
// outside-click both close.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Flag, Terminal, Users } from "lucide-react";

import { api } from "../lib/api";
import type {
  CrewListItem,
  MissionSummary,
  Runner,
} from "../lib/types";

type Kind = "mission" | "runner" | "crew";

interface PaletteItem {
  kind: Kind;
  id: string;
  label: string;
  /** Where Enter / click takes you. */
  navigate: () => void;
  /** Used for substring search alongside the label — adds runner
   *  display_name and crew purpose so query-by-context works. */
  searchText: string;
  /** Order within the kind's bucket when the query is empty. Lower
   *  comes first. Filled in below from the source's recency
   *  ordering. */
  order: number;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

export function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);
  const [items, setItems] = useState<PaletteItem[]>([]);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);

  // Pull the three sources in parallel on open. Cheap enough at v0
  // scale (a few dozen rows total) that we can refetch every time
  // the palette opens — keeps the list fresh without a global
  // subscription. Errors are swallowed: a failed fetch just shows
  // an empty section, and the user can retry by reopening.
  const refresh = useCallback(async () => {
    try {
      const [missions, runners, crews] = await Promise.all([
        api.mission.listSummary().catch(() => [] as MissionSummary[]),
        api.runner.list().catch(() => [] as Runner[]),
        api.crew.list().catch(() => [] as CrewListItem[]),
      ]);
      const next: PaletteItem[] = [];
      missions.forEach((m, i) =>
        next.push({
          kind: "mission",
          id: m.id,
          label: m.title,
          navigate: () => navigate(`/missions/${m.id}`),
          searchText: `${m.title} ${m.crew_name ?? ""}`.toLowerCase(),
          order: i,
        }),
      );
      runners.forEach((r, i) =>
        next.push({
          kind: "runner",
          id: r.id,
          label: `@${r.handle}`,
          navigate: () => navigate(`/runners/${r.handle}`),
          searchText: `${r.handle} ${r.display_name ?? ""}`.toLowerCase(),
          order: i,
        }),
      );
      crews.forEach((c, i) =>
        next.push({
          kind: "crew",
          id: c.id,
          label: c.name,
          navigate: () => navigate(`/crews/${c.id}`),
          searchText: `${c.name} ${c.purpose ?? ""}`.toLowerCase(),
          order: i,
        }),
      );
      setItems(next);
    } catch {
      // Whole-batch failure: leave items empty.
    }
  }, [navigate]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActiveIdx(0);
    void refresh();
    // Auto-focus the input. requestAnimationFrame so the modal has
    // mounted before we steal focus.
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open, refresh]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    const onMouseDown = (e: MouseEvent) => {
      if (!cardRef.current) return;
      if (!cardRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onMouseDown);
    };
  }, [open, onClose]);

  // Filter + group. Sort within each kind by `order` (recency-ish).
  // No-query case shows everything; with-query case substring-
  // matches against `searchText`.
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matched = q
      ? items.filter((it) => it.searchText.includes(q))
      : items.slice();
    matched.sort((a, b) => {
      const kindOrder: Record<Kind, number> = { mission: 0, runner: 1, crew: 2 };
      if (a.kind !== b.kind) return kindOrder[a.kind] - kindOrder[b.kind];
      return a.order - b.order;
    });
    return matched;
  }, [items, query]);

  // Clamp activeIdx whenever the filtered list shrinks — otherwise
  // the highlight points off the end and Enter no-ops.
  useEffect(() => {
    if (activeIdx >= filtered.length && filtered.length > 0) {
      setActiveIdx(0);
    }
  }, [filtered.length, activeIdx]);

  // Scroll the active row into view as the highlight moves so
  // arrow-key navigation through long lists doesn't lose the
  // selection off-screen.
  useEffect(() => {
    if (!open) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-idx="${activeIdx}"]`,
    );
    el?.scrollIntoView({ block: "nearest" });
  }, [activeIdx, open]);

  const onListKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (filtered.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIdx((i) => (i + 1) % filtered.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIdx((i) => (i - 1 + filtered.length) % filtered.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[activeIdx];
      if (item) {
        item.navigate();
        onClose();
      }
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/55 pt-[14vh]">
      <div
        ref={cardRef}
        className="flex w-full max-w-[640px] flex-col overflow-hidden rounded-xl border border-line bg-panel shadow-[0_14px_40px_rgba(0,0,0,0.6)]"
      >
        <div className="flex items-center gap-2.5 border-b border-line px-[18px] py-4">
          <SearchGlyph />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onListKey}
            placeholder="Search…"
            className="flex-1 bg-transparent text-[14px] text-fg outline-none placeholder:text-fg-3"
          />
          <span className="rounded bg-bg px-1.5 py-px font-mono text-[10px] font-medium text-fg-3">
            esc
          </span>
        </div>
        <div className="flex max-h-[420px] flex-col overflow-y-auto py-2">
          {filtered.length === 0 ? (
            <div className="px-3 py-6 text-center text-[12px] text-fg-3">
              {query.trim()
                ? "No matches."
                : "No missions, runners, or crews yet."}
            </div>
          ) : (
            <ul ref={listRef} className="flex flex-col gap-0.5 px-2">
              {!query.trim() ? (
                <li className="px-2.5 py-1.5">
                  <span className="font-mono text-[10px] font-semibold tracking-[1px] text-fg-3">
                    RECENT
                  </span>
                </li>
              ) : null}
              {filtered.map((item, i) => (
                <li key={`${item.kind}:${item.id}`} data-idx={i}>
                  <button
                    type="button"
                    onMouseEnter={() => setActiveIdx(i)}
                    onClick={() => {
                      item.navigate();
                      onClose();
                    }}
                    className={`flex w-full cursor-pointer items-center justify-between gap-3 rounded-md px-2.5 py-2 text-left transition-colors ${
                      i === activeIdx
                        ? "bg-raised text-fg"
                        : "text-fg-2 hover:bg-raised/60 hover:text-fg"
                    }`}
                  >
                    <span className="flex min-w-0 items-center gap-2.5">
                      <KindIcon kind={item.kind} />
                      <span className="truncate text-[13px] font-medium text-fg">
                        {item.label}
                      </span>
                    </span>
                    <span className="font-mono text-[11px] text-fg-3">
                      {item.kind}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

function KindIcon({ kind }: { kind: Kind }) {
  const cls = "h-3.5 w-3.5 text-fg-2";
  if (kind === "mission") return <Flag aria-hidden className={cls} />;
  if (kind === "runner") return <Terminal aria-hidden className={cls} />;
  return <Users aria-hidden className={cls} />;
}

function SearchGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="text-fg-3"
      aria-hidden
    >
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </svg>
  );
}
