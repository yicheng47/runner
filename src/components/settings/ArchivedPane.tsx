// Archived pane — impl 0026. One flat list of archived missions and
// direct chats merged by archive recency, with search (title + cwd)
// and an All | Missions | Chats segmented filter. Unarchive clears
// `archived_at` only; the backend events (`mission/changed` /
// `session/updated`) reinstate the row in the app sidebar live.
// Delete all (title row, feature 01 Phase 4) permanently deletes every
// archived item behind the ConfirmDialog styled per the design's
// `component/confirm-dialog`; the per-row trash is still deferred.

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Archive, MessageSquare, Rocket, Search, Trash2 } from "lucide-react";

import { api, type DirectSessionEntry } from "../../lib/api";
import type { Mission } from "../../lib/types";
import { ConfirmDialog } from "./ConfirmDialog";
import { PaneHeader, SettingsCard } from "./shared";

type TypeFilter = "all" | "missions" | "chats";

const FILTER_OPTIONS: TypeFilter[] = ["all", "missions", "chats"];
const FILTER_LABELS: Record<TypeFilter, string> = {
  all: "All",
  missions: "Missions",
  chats: "Chats",
};

interface ArchivedItem {
  kind: "mission" | "chat";
  /** Mission id or session id — the navigation target's route param. */
  id: string;
  title: string;
  cwd: string | null;
  archivedAt: string;
}

// Same shape as the sidebar's formatStartedAt: same-day → clock time,
// otherwise a short date, so archived rows read like their sidebar
// counterparts did.
function formatTimestamp(ts: string | null): string {
  if (!ts) return "session";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "session";
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// Untitled chats get the sidebar's derived label (`@handle · <time>`)
// so a row archived without a rename is still recognizable here.
function chatTitle(s: DirectSessionEntry): string {
  if (s.title) return s.title;
  const started = formatTimestamp(s.started_at ?? s.stopped_at);
  return s.handle
    ? `@${s.handle} · ${started}`
    : `${s.display_name} · ${started}`;
}

function cwdBasename(cwd: string): string {
  return cwd.replace(/\/+$/, "").split("/").pop() ?? cwd;
}

function byArchivedAtDesc(a: ArchivedItem, b: ArchivedItem): number {
  return Date.parse(b.archivedAt) - Date.parse(a.archivedAt);
}

function toItems(
  missions: Mission[],
  chats: DirectSessionEntry[],
): ArchivedItem[] {
  const items: ArchivedItem[] = [];
  for (const m of missions) {
    if (!m.archived_at) continue;
    items.push({
      kind: "mission",
      id: m.id,
      title: m.title,
      cwd: m.cwd,
      archivedAt: m.archived_at,
    });
  }
  for (const s of chats) {
    if (!s.archived_at) continue;
    items.push({
      kind: "chat",
      id: s.session_id,
      title: chatTitle(s),
      cwd: s.cwd,
      archivedAt: s.archived_at,
    });
  }
  return items.sort(byArchivedAtDesc);
}

export function ArchivedPane() {
  const navigate = useNavigate();
  // null = still loading; the empty state only renders once both
  // fetches have resolved so a fresh pane doesn't flash it.
  const [items, setItems] = useState<ArchivedItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<TypeFilter>("all");
  const [deleting, setDeleting] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([api.mission.listArchived(), api.session.listArchived()])
      .then(([missions, chats]) => {
        if (!cancelled) setItems(toItems(missions, chats));
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const filtered = useMemo(() => {
    if (!items) return [];
    const q = query.trim().toLowerCase();
    return items.filter((item) => {
      if (filter === "missions" && item.kind !== "mission") return false;
      if (filter === "chats" && item.kind !== "chat") return false;
      if (!q) return true;
      return (
        item.title.toLowerCase().includes(q) ||
        (item.cwd ?? "").toLowerCase().includes(q)
      );
    });
  }, [items, query, filter]);

  // Optimistic: drop the row immediately, restore it (re-sorted) if
  // the backend refuses.
  const unarchive = async (item: ArchivedItem) => {
    setItems(
      (prev) =>
        prev?.filter((i) => !(i.kind === item.kind && i.id === item.id)) ??
        prev,
    );
    try {
      if (item.kind === "mission") await api.mission.unarchive(item.id);
      else await api.session.unarchive(item.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setItems((prev) =>
        prev ? [...prev, item].sort(byArchivedAtDesc) : prev,
      );
    }
  };

  const open = (item: ArchivedItem) => {
    navigate(item.kind === "mission" ? `/missions/${item.id}` : `/chats/${item.id}`);
  };

  // Deletes everything archived, not just the filtered view — the
  // dialog body names the full count so the scope is unmissable.
  // Failed rows stay listed with the first error surfaced. The
  // `deleting` guard covers the confirm-to-done window: a second
  // submit would replay the same stale `items` and resurrect
  // already-deleted rows as not-found failures.
  const runDeleteAll = async () => {
    if (deleting || !items || items.length === 0) return;
    setDeleting(true);
    try {
      const failed: ArchivedItem[] = [];
      let firstError: string | null = null;
      for (const item of items) {
        try {
          if (item.kind === "mission") await api.mission.delete(item.id);
          else await api.session.delete(item.id);
        } catch (e) {
          failed.push(item);
          firstError ??= e instanceof Error ? e.message : String(e);
        }
      }
      setItems(failed);
      setError(firstError);
    } finally {
      setDeleting(false);
      setConfirmOpen(false);
    }
  };

  return (
    <>
      <PaneHeader
        title="Archived chats & missions"
        subtitle="Everything you've archived — restore anytime, or delete permanently."
        action={
          <button
            type="button"
            onClick={() => setConfirmOpen(true)}
            disabled={deleting || !items || items.length === 0}
            className="flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-lg border border-danger/40 bg-danger/10 px-3 text-[12px] font-medium text-danger transition-colors hover:bg-danger/20 disabled:cursor-default disabled:opacity-40 disabled:hover:bg-danger/10"
          >
            <Trash2 aria-hidden className="h-3.5 w-3.5" />
            Delete all
          </button>
        }
      />
      <div className="flex items-center gap-3">
        <div className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md border border-line bg-bg px-2.5">
          <Search aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search archived…"
            className="min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-fg-3"
          />
        </div>
        <TypeSegmented value={filter} onChange={setFilter} />
      </div>
      {error ? (
        <div className="rounded-xl border border-red-500/30 bg-red-500/10 px-4 py-3">
          <p className="text-[12px] text-red-300">{error}</p>
        </div>
      ) : null}
      {items == null ? null : items.length === 0 ? (
        <div className="flex flex-col items-center gap-1.5 rounded-xl border border-line bg-panel px-6 py-12 text-center">
          <Archive aria-hidden className="h-5 w-5 text-fg-3" />
          <p className="mt-1 text-[13px] font-medium text-fg">
            Nothing archived yet
          </p>
          <p className="text-[12px] text-fg-2">
            Archive a chat or mission from the sidebar and it will land here.
          </p>
        </div>
      ) : (
        <SettingsCard>
          {filtered.length === 0 ? (
            <p className="px-4 py-6 text-center text-[12px] text-fg-3">
              No archived items match.
            </p>
          ) : (
            filtered.map((item) => (
              <ArchivedRow
                key={`${item.kind}:${item.id}`}
                item={item}
                onOpen={() => open(item)}
                onUnarchive={() => void unarchive(item)}
              />
            ))
          )}
        </SettingsCard>
      )}
      <ConfirmDialog
        open={confirmOpen}
        title="Delete all archived items?"
        body={`This permanently deletes all ${items?.length ?? 0} archived ${
          items?.length === 1 ? "item" : "items"
        }, including mission event logs. This can't be undone.`}
        confirmLabel="Delete all"
        busyLabel="Deleting…"
        busy={deleting}
        onConfirm={() => void runDeleteAll()}
        onCancel={() => setConfirmOpen(false)}
      />
    </>
  );
}

function ArchivedRow({
  item,
  onOpen,
  onUnarchive,
}: {
  item: ArchivedItem;
  onOpen: () => void;
  onUnarchive: () => void;
}) {
  const Icon = item.kind === "mission" ? Rocket : MessageSquare;
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen();
      }}
      title={item.cwd ?? undefined}
      className="flex cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-raised/40 focus:bg-raised/40 focus:outline-none"
    >
      <Icon aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
      <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-fg">
        {item.title}
      </span>
      {item.cwd ? (
        <span className="max-w-[180px] shrink-0 truncate font-mono text-[11px] text-fg-3">
          {cwdBasename(item.cwd)}
        </span>
      ) : null}
      <span className="shrink-0 text-[11px] text-fg-3">
        {formatTimestamp(item.archivedAt)}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onUnarchive();
        }}
        className="shrink-0 cursor-pointer rounded-md border border-line bg-raised px-2.5 py-1 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg"
      >
        Unarchive
      </button>
    </div>
  );
}

// All | Missions | Chats. Same cell treatment as the Appearance pane's
// ThemeSegmented — active cell lifts onto the raised surface.
function TypeSegmented({
  value,
  onChange,
}: {
  value: TypeFilter;
  onChange: (next: TypeFilter) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label="Type"
      className="flex shrink-0 items-center gap-0.5 rounded-md border border-line bg-bg p-0.5"
    >
      {FILTER_OPTIONS.map((option) => {
        const active = option === value;
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option)}
            className={`cursor-pointer rounded-[4px] px-2.5 py-[5px] text-[12px] font-medium transition-colors ${
              active ? "bg-raised text-fg" : "text-fg-2 hover:text-fg"
            }`}
          >
            {FILTER_LABELS[option]}
          </button>
        );
      })}
    </div>
  );
}
