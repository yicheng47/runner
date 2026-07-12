// Direct-chat split-view layout model (impl 0020, spec 34).
//
// A layout is a binary tree of splits with 1–3 leaf panes, built only from
// the six picker presets (1 · 2 side-by-side · 2 stacked · 1-big+2-stacked ·
// 3 columns · 3 rows). Each leaf maps to at most one direct-chat session —
// a session lives in exactly one pane (move-not-copy), which is what keeps
// the single-writer stdin invariant: RunnerChat mounts one RunnerTerminal
// per session, ever, and the layout only decides which of them are visible.
//
// State is a module-level snapshot shared by RunnerChat and Sidebar via
// `useSyncExternalStore`. SQLite is authoritative for tab identity, folder
// membership, names, order, and layouts; every window writes through and
// rehydrates on `chat/layout-changed` invalidations.

import { useSyncExternalStore } from "react";

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, type FolderRow, type TabRow } from "./api";

export type PresetKind =
  | "single"
  | "cols-2"
  | "rows-2"
  | "main-2"
  | "cols-3"
  | "rows-3";

export interface PaneLeaf {
  kind: "leaf";
  id: string;
  /** Direct-chat session shown in this pane; null = empty pane. */
  sessionId: string | null;
}

export interface PaneSplit {
  kind: "split";
  /** Namespaced by preset so switching presets remounts the panel groups
   *  and their default sizes re-apply. */
  id: string;
  /** row = panes side by side; col = panes stacked. */
  orientation: "row" | "col";
  /** Percentage sizes of `a` and `b`. Mutated in place on gutter drag via
   *  `recordSplitSizes` (non-reactive — the panel lib owns live sizes). */
  sizes: [number, number];
  a: PaneNode;
  b: PaneNode;
}

export type PaneNode = PaneLeaf | PaneSplit;

export interface PaneLayout {
  /** Stable SQLite-backed tab identity. Empty only for the off-Tauri test
   *  placeholder before the first hydration. */
  id: string;
  folderId: string | null;
  preset: PresetKind;
  root: PaneNode;
  focusedPaneId: string;
  /** User-given group name; null = derive from member chat names. Only
   *  meaningful while split — a fresh group starts unnamed. */
  name: string | null;
}

// ---- pure helpers -------------------------------------------------------

/** Leaves in slot order (depth-first, `a` before `b`). Slot 0 is the
 *  preset's biggest pane. */
export function leaves(node: PaneNode): PaneLeaf[] {
  if (node.kind === "leaf") return [node];
  return [...leaves(node.a), ...leaves(node.b)];
}

export function findLeaf(node: PaneNode, paneId: string): PaneLeaf | null {
  return leaves(node).find((l) => l.id === paneId) ?? null;
}

export function leafForSession(
  node: PaneNode,
  sessionId: string,
): PaneLeaf | null {
  return leaves(node).find((l) => l.sessionId === sessionId) ?? null;
}

/** Sessions currently on screen, in slot order. */
export function visibleSessionIds(node: PaneNode): string[] {
  return leaves(node)
    .map((l) => l.sessionId)
    .filter((s): s is string => s !== null);
}

/**
 * Whether the split group renders for the given chat: the layout is a
 * binding between member sessions, not a viewport mode, so it only shows
 * while the open chat is one of its members (decision 6). Non-member
 * chats render classic single-pane over an intact background group.
 */
export function isGroupActiveFor(
  layout: PaneLayout,
  sessionId: string | null,
): boolean {
  return (
    layout.root.kind === "split" &&
    sessionId !== null &&
    leafForSession(layout.root, sessionId) !== null
  );
}

/** The first empty pane a sidebar-created new chat should fill, or null
 *  when the current chat does not have an active split group on screen. */
export function newChatTargetPane(
  layout: PaneLayout,
  currentChatSessionId: string | null,
): string | null {
  if (!isGroupActiveFor(layout, currentChatSessionId)) return null;
  return leaves(layout.root).find((l) => l.sessionId === null)?.id ?? null;
}

function leaf(id: string, sessionId: string | null): PaneLeaf {
  return { kind: "leaf", id, sessionId };
}

function split(
  id: string,
  orientation: "row" | "col",
  sizes: [number, number],
  a: PaneNode,
  b: PaneNode,
): PaneSplit {
  return { kind: "split", id, orientation, sizes, a, b };
}

export function paneCountFor(kind: PresetKind): number {
  switch (kind) {
    case "single":
      return 1;
    case "cols-2":
    case "rows-2":
      return 2;
    case "main-2":
    case "cols-3":
    case "rows-3":
      return 3;
  }
}

function buildPresetTree(
  kind: PresetKind,
  s: (string | null)[],
): PaneNode {
  const p1 = leaf("p1", s[0] ?? null);
  const p2 = leaf("p2", s[1] ?? null);
  const p3 = leaf("p3", s[2] ?? null);
  switch (kind) {
    case "single":
      return p1;
    case "cols-2":
      return split(`${kind}:outer`, "row", [50, 50], p1, p2);
    case "rows-2":
      return split(`${kind}:outer`, "col", [50, 50], p1, p2);
    case "main-2":
      return split(
        `${kind}:outer`,
        "row",
        [60, 40],
        p1,
        split(`${kind}:inner`, "col", [50, 50], p2, p3),
      );
    case "cols-3":
      return split(
        `${kind}:outer`,
        "row",
        [33.33, 66.67],
        p1,
        split(`${kind}:inner`, "row", [50, 50], p2, p3),
      );
    case "rows-3":
      return split(
        `${kind}:outer`,
        "col",
        [33.33, 66.67],
        p1,
        split(`${kind}:inner`, "col", [50, 50], p2, p3),
      );
  }
}

/**
 * Build a preset layout, filling slots from the currently visible chats:
 * the focused chat keeps the biggest slot (slot 0), the rest follow in
 * their current slot order, and leftover slots stay empty. Focus lands on
 * the first empty pane when the preset has more slots than open chats
 * (the caller auto-opens StartChatModal for it), else on the focused
 * chat's pane.
 */
export function applyPresetPure(
  kind: PresetKind,
  focusedSessionId: string | null,
  currentVisible: string[],
  name: string | null = null,
): PaneLayout {
  const rest = currentVisible.filter((s) => s !== focusedSessionId);
  const ordered = focusedSessionId ? [focusedSessionId, ...rest] : rest;
  const slots = ordered.slice(0, paneCountFor(kind));
  const root = buildPresetTree(kind, slots);
  const all = leaves(root);
  const firstEmpty = all.find((l) => l.sessionId === null);
  const focusedLeaf = focusedSessionId
    ? all.find((l) => l.sessionId === focusedSessionId)
    : null;
  return {
    id: "",
    folderId: null,
    preset: kind,
    root,
    focusedPaneId: (firstEmpty ?? focusedLeaf ?? all[0]).id,
    name: kind === "single" ? null : name,
  };
}

/**
 * Load a session into a pane, move-not-copy: if the session is already
 * visible in another pane, that pane is cleared to empty so exactly one
 * pane ever shows a given chat.
 */
export function assignSessionPure(
  layout: PaneLayout,
  paneId: string,
  sessionId: string,
): PaneLayout {
  const map = (node: PaneNode): PaneNode => {
    if (node.kind === "leaf") {
      if (node.id === paneId) return { ...node, sessionId };
      if (node.sessionId === sessionId) return { ...node, sessionId: null };
      return node;
    }
    return { ...node, a: map(node.a), b: map(node.b) };
  };
  return { ...layout, root: map(layout.root) };
}

/** Clear a session out of whatever pane shows it (archive path). The pane
 *  stays, empty. No-op (same object) when the session isn't visible. */
export function removeSessionPure(
  layout: PaneLayout,
  sessionId: string,
): PaneLayout {
  if (!leafForSession(layout.root, sessionId)) return layout;
  const map = (node: PaneNode): PaneNode => {
    if (node.kind === "leaf") {
      return node.sessionId === sessionId ? { ...node, sessionId: null } : node;
    }
    return { ...node, a: map(node.a), b: map(node.b) };
  };
  return { ...layout, root: map(layout.root) };
}

/** Derive the picker highlight after a collapse reshapes the tree. */
function derivePreset(root: PaneNode): PresetKind {
  if (root.kind === "leaf") return "single";
  const { orientation, a, b } = root;
  if (a.kind === "leaf" && b.kind === "leaf") {
    return orientation === "row" ? "cols-2" : "rows-2";
  }
  if (b.kind === "split") {
    if (orientation === "row" && b.orientation === "col") return "main-2";
    if (orientation === "row") return "cols-3";
    return "rows-3";
  }
  // b leaf, a split — collapse never produces this from preset trees, but
  // classify by orientation anyway rather than lying with "single".
  return orientation === "row" ? "cols-3" : "rows-3";
}

/**
 * Collapse a pane: remove its leaf and promote the sibling into the
 * parent's slot. The pane's session (if any) drops back to the hidden
 * stack — closing a pane never stops the session (key decision 8). Focus
 * moves to the promoted subtree's first leaf when the closed pane was
 * focused.
 */
export function closePanePure(layout: PaneLayout, paneId: string): PaneLayout {
  if (layout.root.kind === "leaf") return layout;

  const remove = (node: PaneNode): PaneNode | null => {
    if (node.kind === "leaf") return node.id === paneId ? null : node;
    const a = remove(node.a);
    const b = remove(node.b);
    if (a === null) return b;
    if (b === null) return a;
    if (a === node.a && b === node.b) return node;
    return { ...node, a, b };
  };

  const root = remove(layout.root);
  if (root === null || root === layout.root) return layout;
  const all = leaves(root);
  const focusedPaneId = all.some((l) => l.id === layout.focusedPaneId)
    ? layout.focusedPaneId
    : all[0].id;
  const preset = derivePreset(root);
  return {
    id: layout.id,
    folderId: layout.folderId,
    preset,
    root,
    focusedPaneId,
    name: preset === "single" ? null : layout.name,
  };
}

export function setSizesPure(
  layout: PaneLayout,
  splitId: string,
  sizes: [number, number],
): PaneLayout {
  const map = (node: PaneNode): PaneNode => {
    if (node.kind === "leaf") return node;
    if (node.id === splitId) return { ...node, sizes };
    return { ...node, a: map(node.a), b: map(node.b) };
  };
  return { ...layout, root: map(layout.root) };
}

// ---- persistence --------------------------------------------------------

const STORAGE_LAYOUT = "runner.chat.layout";

const PRESET_KINDS: readonly PresetKind[] = [
  "single",
  "cols-2",
  "rows-2",
  "main-2",
  "cols-3",
  "rows-3",
];

function isPresetKind(v: unknown): v is PresetKind {
  return typeof v === "string" && (PRESET_KINDS as string[]).includes(v);
}

// Storage schema is deliberately not the tree itself: preset + slot
// assignments + per-split sizes + focused slot index. Restoring rebuilds
// the tree through the same preset builder the picker uses, so a stale or
// hand-edited payload can never produce a shape the renderer hasn't seen.
interface PersistedLayout {
  preset: PresetKind;
  slots: (string | null)[];
  sizes: Record<string, [number, number]>;
  /** Legacy localStorage-only view state, read during the one-time import. */
  focusedSlot?: number;
  /** Legacy localStorage name, moved to the tabs.name column on import. */
  name?: string | null;
  /** Legacy impl 0023 accordion state. Ignored after import. */
  collapsed?: boolean;
}

interface PersistedLayoutSet {
  version: 2;
  activeSlot: number;
  tabs: PersistedLayout[];
}

function toPersistedLayout(
  layout: PaneLayout,
  includeLegacyViewState = false,
): PersistedLayout {
  const all = leaves(layout.root);
  const sizes: Record<string, [number, number]> = {};
  const walk = (node: PaneNode): void => {
    if (node.kind === "leaf") return;
    sizes[node.id] = node.sizes;
    walk(node.a);
    walk(node.b);
  };
  walk(layout.root);
  const persisted: PersistedLayout = {
    preset: layout.preset,
    slots: all.map((l) => l.sessionId),
    sizes,
  };
  if (includeLegacyViewState) {
    persisted.focusedSlot = Math.max(
      0,
      all.findIndex((leaf) => leaf.id === layout.focusedPaneId),
    );
    persisted.name = layout.name;
  }
  return persisted;
}

function fromPersistedLayout(
  p: Partial<PersistedLayout> | null,
): PaneLayout | null {
  if (!p || !isPresetKind(p.preset)) return null;
  const slots = Array.isArray(p.slots)
    ? p.slots.map((s) => (typeof s === "string" ? s : null))
    : [];
  const root = buildPresetTree(p.preset, slots);
  const applySizes = (node: PaneNode): void => {
    if (node.kind === "leaf") return;
    const stored = p.sizes?.[node.id];
    if (
      Array.isArray(stored) &&
      stored.length === 2 &&
      stored.every((n) => typeof n === "number" && n > 0 && n < 100)
    ) {
      node.sizes = [stored[0], stored[1]];
    }
    applySizes(node.a);
    applySizes(node.b);
  };
  applySizes(root);
  const all = leaves(root);
  const focusedSlot =
    typeof p.focusedSlot === "number" &&
    Number.isInteger(p.focusedSlot) &&
    p.focusedSlot >= 0 &&
    p.focusedSlot < all.length
      ? p.focusedSlot
      : 0;
  return {
    id: "",
    folderId: null,
    preset: p.preset,
    root,
    focusedPaneId: all[focusedSlot].id,
    name: typeof p.name === "string" && p.name.length > 0 ? p.name : null,
  };
}

export function serializeLayout(layout: PaneLayout): string {
  return JSON.stringify(toPersistedLayout(layout));
}

/** Rebuild a layout from a persisted payload; null on any malformed or
 *  unrecognized input (callers fall back to a fresh single pane). */
export function deserializeLayout(raw: string): PaneLayout | null {
  try {
    return fromPersistedLayout(
      JSON.parse(raw) as Partial<PersistedLayout> | null,
    );
  } catch {
    return null;
  }
}

// This window's Tauri label (`main` or `window-<ulid>`), resolved once at
// module load. Null off-Tauri (dev browser preview), where the cross-window
// broadcast/listen below all no-op.
let windowLabel: string | null = null;
try {
  windowLabel = getCurrentWindow().label;
} catch {
  windowLabel = null;
}

// ---- module store -------------------------------------------------------

function singleLayout(sessionId: string | null = null): PaneLayout {
  return {
    id: "",
    folderId: null,
    preset: "single",
    root: leaf("p1", sessionId),
    focusedPaneId: "p1",
    name: null,
  };
}

function deserializeLayoutSet(
  raw: string,
): { layouts: PaneLayout[]; activeIndex: number } | null {
  try {
    const parsed = JSON.parse(raw) as Partial<PersistedLayoutSet> | null;
    if (parsed?.version !== 2 || !Array.isArray(parsed.tabs)) {
      const legacy = fromPersistedLayout(
        parsed as Partial<PersistedLayout> | null,
      );
      return legacy ? { layouts: [legacy], activeIndex: 0 } : null;
    }
    const layouts = parsed.tabs
      .map((tab) => fromPersistedLayout(tab))
      .filter((tab): tab is PaneLayout => tab !== null);
    if (layouts.length === 0) return null;
    const activeIndex =
      typeof parsed.activeSlot === "number" &&
      Number.isInteger(parsed.activeSlot) &&
      parsed.activeSlot >= 0 &&
      parsed.activeSlot < layouts.length
        ? parsed.activeSlot
        : 0;
    return { layouts, activeIndex };
  } catch {
    return null;
  }
}

export function serializeLayoutSet(
  layouts: PaneLayout[],
  activeIndex: number,
): string {
  return JSON.stringify({
    version: 2,
    activeSlot: activeIndex,
    tabs: layouts.map((layout) => toPersistedLayout(layout, true)),
  } satisfies PersistedLayoutSet);
}

// Read the legacy localStorage set only for the one-time SQLite import.
function readPersistedLayoutSet(): {
  layouts: PaneLayout[];
  activeIndex: number;
} | null {
  try {
    const raw = localStorage.getItem(STORAGE_LAYOUT);
    return raw ? deserializeLayoutSet(raw) : null;
  } catch {
    return null;
  }
}

function normalizeLayoutSet(
  nextLayouts: PaneLayout[],
  nextActiveIndex: number,
): { layouts: PaneLayout[]; activeIndex: number } {
  const source = nextLayouts.length > 0 ? nextLayouts : [singleLayout()];
  const keep = source
    .map((layout, index) => ({ layout, index }))
    .filter(
      ({ layout, index }) =>
        index === nextActiveIndex || visibleSessionIds(layout.root).length > 0,
    );
  const entries =
    keep.length > 0 ? keep : [{ layout: singleLayout(), index: 0 }];
  const active = Math.max(
    0,
    entries.findIndex(({ index }) => index === nextActiveIndex),
  );
  return {
    layouts: entries.map(({ layout }) => layout),
    activeIndex: active,
  };
}

let layouts: PaneLayout[] = [singleLayout()];
let activeIndex = 0;
let folders: FolderRow[] = [];
const listeners = new Set<() => void>();
let writeQueue: Promise<unknown> = Promise.resolve();
let hydrationSequence = 0;

// The chat this window owns via its route — the last non-null session passed
// to `setRouteAnchorSession` (only RunnerChat's URL effect calls it). This is
// the anchor for cross-window hydration: it is local to this window and,
// unlike a tab's `focusedPaneId`, never carried in the synced payload, so a
// remote change can't move it off the tab it renders.
let routeAnchorSession: string | null = null;

function currentLayout(): PaneLayout {
  return layouts[activeIndex] ?? layouts[0] ?? singleLayout();
}

function findLayoutIndexForSession(sessionId: string | null): number {
  if (!sessionId) return -1;
  return layouts.findIndex((layout) => leafForSession(layout.root, sessionId));
}

// Commit a new layout set to the in-window store: normalize, swap in fresh
// arrays, and notify subscribers. DB hydration reuses this without writing.
function applyLayoutSet(
  nextLayouts: PaneLayout[],
  nextActiveIndex: number,
): void {
  const normalized = normalizeLayoutSet(nextLayouts, nextActiveIndex);
  layouts = normalized.layouts;
  activeIndex = normalized.activeIndex;
  for (const l of listeners) l();
}

function layoutInput(layout: PaneLayout, position: number) {
  return {
    id: layout.id,
    folder_id: layout.folderId,
    name: normalizedGroupName(layout.name) ?? "",
    position,
    layout: serializeLayout(layout),
  };
}

function enqueueWrite(write: () => Promise<unknown>, label: string): void {
  writeQueue = writeQueue
    .catch(() => undefined)
    .then(write)
    .catch((e) => console.error(`paneLayout: ${label} failed`, e));
}

function writeLayoutChanges(previous: PaneLayout[]): void {
  if (windowLabel === null) return;
  const nextIds = new Set(layouts.map((layout) => layout.id).filter(Boolean));
  for (const prior of previous) {
    if (prior.id && !nextIds.has(prior.id)) {
      enqueueWrite(() => api.tab.delete(prior.id), "tab_delete");
    }
  }
  layouts.forEach((layout, position) => {
    if (!layout.id) return;
    const before = previous.find((candidate) => candidate.id === layout.id);
    if (
      before &&
      before.folderId === layout.folderId &&
      before.name === layout.name &&
      serializeLayout(before) === serializeLayout(layout) &&
      previous.indexOf(before) === position
    ) {
      return;
    }
    const input = layoutInput(layout, position);
    enqueueWrite(() => api.tab.upsert(input), "tab_upsert");
  });
}

function setLayoutSet(
  nextLayouts: PaneLayout[],
  nextActiveIndex: number,
): void {
  const previous = layouts;
  applyLayoutSet(nextLayouts, nextActiveIndex);
  writeLayoutChanges(previous);
}

function setCurrent(next: PaneLayout): void {
  if (next === currentLayout()) return;
  const nextLayouts = [...layouts];
  nextLayouts[activeIndex] = next;
  setLayoutSet(nextLayouts, activeIndex);
}

// ---- DB hydration + cross-window sync -----------------------------------

const LAYOUT_CHANGED_EVENT = "chat/layout-changed";

/**
 * Apply a layout set received from another window: hydrate the in-memory
 * store and notify subscribers, without re-broadcasting (echo guard). No-op
 * and returns false on a malformed payload. Exported for the sync listener
 * and its unit test.
 */
export function hydrateLayoutSet(raw: string): boolean {
  const parsed = deserializeLayoutSet(raw);
  if (!parsed) return false;

  // Tab membership / sizes / collapsed / names converge, but activeIndex is
  // per-window view state bound to this window's route. Adopting the
  // sender's would repoint `currentLayout()` away from the tab this window
  // renders, so a local `closePane`/`focusPane`/`setGroupName` would mutate
  // the wrong tab. Keep our own active tab: re-find, in the incoming set,
  // the tab showing our route session (`routeAnchorSession` — local, and
  // not part of the synced payload, unlike a tab's focused pane). Fall back
  // to clamping our index into range when we don't own a route yet.
  const anchor = routeAnchorSession;
  const anchoredIndex =
    anchor !== null
      ? parsed.layouts.findIndex((l) => leafForSession(l.root, anchor))
      : -1;
  const nextActive =
    anchoredIndex >= 0
      ? anchoredIndex
      : Math.min(activeIndex, parsed.layouts.length - 1);

  // Members may be sessions this window's row cache hasn't observed yet (a
  // chat just created in another window). Mark them fresh so RunnerChat's
  // vanished-session sweep leaves them alone until the rows catch up,
  // instead of evicting them and broadcasting the stale layout back.
  for (const l of parsed.layouts) {
    for (const sid of visibleSessionIds(l.root)) {
      freshAssignments.set(sid, Date.now());
    }
  }

  applyLayoutSet(parsed.layouts, nextActive);
  return true;
}

function layoutFromTabRow(row: TabRow): PaneLayout | null {
  const parsed = deserializeLayout(row.layout);
  if (!parsed) return null;
  return {
    ...parsed,
    id: row.id,
    folderId: row.folder_id,
    name: row.name.trim() || null,
  };
}

function importTabsFromLocalStorage(): { name: string; position: number; layout: string }[] {
  const persisted = readPersistedLayoutSet();
  if (!persisted) return [];
  return persisted.layouts.map((layout, position) => ({
    name: layout.name ?? "",
    position,
    layout: serializeLayout(layout),
  }));
}

export async function hydratePaneLayoutsFromDb(): Promise<void> {
  if (windowLabel === null) return;
  const sequence = ++hydrationSequence;
  const imported = importTabsFromLocalStorage();
  const [tabRows, folderRows] = await Promise.all([
    imported.length > 0 ? api.tab.importOnce(imported) : api.tab.list(),
    api.folder.list(),
  ]);
  if (sequence !== hydrationSequence) return;
  if (imported.length > 0) {
    try {
      localStorage.removeItem(STORAGE_LAYOUT);
    } catch {
      // Import succeeded; stale browser storage is harmless if unavailable.
    }
  }
  const next = tabRows
    .map(layoutFromTabRow)
    .filter((layout): layout is PaneLayout => layout !== null)
    .map((layout) => {
      const prior = layouts.find((candidate) => candidate.id === layout.id);
      const focusedSession = prior
        ? findLeaf(prior.root, prior.focusedPaneId)?.sessionId
        : null;
      const focusedLeaf = focusedSession
        ? leafForSession(layout.root, focusedSession)
        : null;
      return focusedLeaf ? { ...layout, focusedPaneId: focusedLeaf.id } : layout;
    });
  const anchorIndex = routeAnchorSession
    ? next.findIndex((layout) => leafForSession(layout.root, routeAnchorSession!))
    : -1;
  const currentId = currentLayout().id;
  const currentIndex = next.findIndex((layout) => layout.id === currentId);
  folders = folderRows;
  applyLayoutSet(
    next.length > 0 ? next : [singleLayout()],
    anchorIndex >= 0 ? anchorIndex : currentIndex >= 0 ? currentIndex : 0,
  );
}

if (windowLabel !== null) {
  void hydratePaneLayoutsFromDb().catch((e) => {
    console.error("paneLayout: initial hydration failed", e);
  });
  void listen(LAYOUT_CHANGED_EVENT, () => {
    void hydratePaneLayoutsFromDb().catch((e) =>
      console.error("paneLayout: cross-window hydration failed", e),
    );
  }).catch(() => {
    // Tauri unavailable — cross-window sync simply no-ops.
  });
}

export function getPaneLayout(sessionId: string | null = null): PaneLayout {
  const index = findLayoutIndexForSession(sessionId);
  return index >= 0 ? layouts[index] : currentLayout();
}

export function subscribePaneLayout(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function usePaneLayout(sessionId: string | null = null): PaneLayout {
  return useSyncExternalStore(
    subscribePaneLayout,
    () => getPaneLayout(sessionId),
    () => getPaneLayout(sessionId),
  );
}

/** The whole tab set, in tab order. The reference is stable between store
 *  updates (each mutation swaps in a fresh array), so it is a sound
 *  `useSyncExternalStore` snapshot for the sidebar's grouped render. */
export function getPaneLayouts(): PaneLayout[] {
  return layouts;
}

export function usePaneLayouts(): PaneLayout[] {
  return useSyncExternalStore(
    subscribePaneLayout,
    getPaneLayouts,
    getPaneLayouts,
  );
}

export function getFolders(): FolderRow[] {
  return folders;
}

export function useFolders(): FolderRow[] {
  return useSyncExternalStore(subscribePaneLayout, getFolders, getFolders);
}

export async function createChatFolder(name: string): Promise<FolderRow> {
  const row = await api.folder.create(name);
  // A refresh started before folder_create committed must not overwrite the
  // authoritative row returned by this command with its older folder list.
  hydrationSequence += 1;
  folders = [...folders.filter((folder) => folder.id !== row.id), row].sort(
    (a, b) => a.position - b.position,
  );
  for (const listener of listeners) listener();
  try {
    await hydratePaneLayoutsFromDb();
  } catch (error) {
    console.error("paneLayout: post-create hydration failed", error);
  }
  return row;
}

export async function moveTabToFolder(
  tabId: string,
  folderId: string | null,
): Promise<void> {
  const index = layouts.findIndex((layout) => layout.id === tabId);
  if (index < 0) return;
  const next = [...layouts];
  next[index] = { ...next[index], folderId };
  applyLayoutSet(next, activeIndex);
  await api.tab.moveToFolder(tabId, folderId);
}

export async function reorderTab(
  tabId: string,
  folderId: string | null,
  orderedIds: string[],
): Promise<void> {
  const dragged = layouts.find((layout) => layout.id === tabId);
  if (!dragged) return;
  const previousLayouts = layouts;
  const activeId = currentLayout().id;
  const targetIds = new Set(orderedIds);
  if (!targetIds.has(tabId)) throw new Error("reorder must include dragged tab");
  const ordered = orderedIds.map((id) => {
    const layout = layouts.find((candidate) => candidate.id === id);
    if (!layout) throw new Error(`tab not found: ${id}`);
    return id === tabId ? { ...layout, folderId } : layout;
  });
  const firstTargetIndex = layouts.findIndex((layout) =>
    targetIds.has(layout.id),
  );
  const insertionIndex = layouts
    .slice(0, Math.max(0, firstTargetIndex))
    .filter((layout) => !targetIds.has(layout.id)).length;
  const next = layouts.filter((layout) => !targetIds.has(layout.id));
  next.splice(insertionIndex, 0, ...ordered);
  applyLayoutSet(
    next,
    Math.max(
      0,
      next.findIndex((layout) => layout.id === activeId),
    ),
  );
  const optimisticLayouts = layouts;
  try {
    await api.tab.reorder(tabId, folderId, orderedIds);
  } catch (error) {
    if (layouts === optimisticLayouts) {
      applyLayoutSet(
        previousLayouts,
        Math.max(
          0,
          previousLayouts.findIndex((layout) => layout.id === activeId),
        ),
      );
    }
    await hydratePaneLayoutsFromDb().catch(() => undefined);
    throw error;
  }
  try {
    await hydratePaneLayoutsFromDb();
  } catch (error) {
    console.error("paneLayout: post-reorder hydration failed", error);
  }
}

export async function moveSessionTabToFolder(
  sessionId: string,
  folderId: string,
): Promise<void> {
  const rows = await api.tab.list();
  const row = rows.find((candidate) => {
    const layout = layoutFromTabRow(candidate);
    return layout ? leafForSession(layout.root, sessionId) !== null : false;
  });
  if (!row) throw new Error(`tab not found for session: ${sessionId}`);
  const moved = layoutFromTabRow(
    await api.tab.moveToFolder(row.id, folderId),
  );
  if (moved) {
    const index = layouts.findIndex((candidate) => candidate.id === moved.id);
    const next = [...layouts];
    if (index >= 0) next[index] = moved;
    else next.push(moved);
    applyLayoutSet(next, activeIndex);
  }
  try {
    await hydratePaneLayoutsFromDb();
  } catch (error) {
    console.error("paneLayout: post-move hydration failed", error);
  }
}

/**
 * Record the chat this window owns via its route, for cross-window hydration
 * to anchor on. Only RunnerChat's URL effect should call this — generic tab
 * activations (a sidebar pick, renaming a background tab) reactivate a tab
 * without owning it as the route, and must not move the anchor. Keeps the
 * last non-null id so navigating to a non-chat surface doesn't drop it.
 */
export function setRouteAnchorSession(sessionId: string | null): void {
  if (sessionId !== null) routeAnchorSession = sessionId;
}

export function activatePaneLayoutForSession(
  sessionId: string | null,
): PaneLayout {
  const index = findLayoutIndexForSession(sessionId);
  if (index >= 0) {
    const memberLeaf = sessionId
      ? leafForSession(layouts[index].root, sessionId)
      : null;
    const nextLayouts = memberLeaf
      ? layouts.map((layout, layoutIndex) =>
          layoutIndex === index
            ? { ...layout, focusedPaneId: memberLeaf.id }
            : layout,
        )
      : layouts;
    if (index !== activeIndex || nextLayouts !== layouts) {
      applyLayoutSet(nextLayouts, index);
    }
    return currentLayout();
  }
  return currentLayout();
}

export function resetPaneLayoutsForTest(
  nextLayouts: PaneLayout[] = [singleLayout()],
  nextActiveIndex = 0,
): void {
  const normalized = normalizeLayoutSet(nextLayouts, nextActiveIndex);
  layouts = normalized.layouts;
  activeIndex = normalized.activeIndex;
  routeAnchorSession = null;
  freshAssignments.clear();
  for (const l of listeners) l();
}

export function getPaneLayoutsForTest(): PaneLayout[] {
  return layouts;
}

export function applyPreset(
  kind: PresetKind,
  focusedSessionId: string | null,
  currentVisible: string[],
  name: string | null = null,
): PaneLayout {
  const next = applyPresetPure(kind, focusedSessionId, currentVisible, name);
  const existingIndex = findLayoutIndexForSession(focusedSessionId);
  if (existingIndex >= 0) {
    const nextLayouts = [...layouts];
    nextLayouts[existingIndex] = {
      ...next,
      id: layouts[existingIndex].id,
      folderId: layouts[existingIndex].folderId,
    };
    setLayoutSet(nextLayouts, existingIndex);
    return currentLayout();
  }
  const active = currentLayout();
  if (
    active.preset === "single" &&
    visibleSessionIds(active.root).length <= 1
  ) {
    setCurrent({ ...next, id: active.id, folderId: active.folderId });
    return currentLayout();
  }
  setLayoutSet([...layouts, next], layouts.length);
  return currentLayout();
}

function normalizedGroupName(name: string | null): string | null {
  return name?.trim() || null;
}

function setGroupNameAtIndex(index: number, name: string | null): void {
  if (index < 0 || index >= layouts.length) return;
  const trimmed = normalizedGroupName(name);
  const layout = layouts[index];
  if (layout.name === trimmed) return;
  const nextLayouts = [...layouts];
  nextLayouts[index] = { ...layout, name: trimmed };
  setLayoutSet(nextLayouts, activeIndex);
}

/** Name (or un-name, with null/blank) the active split group. */
export function setGroupName(name: string | null): void {
  setGroupNameAtIndex(activeIndex, name);
}

/** Name (or un-name, with null/blank) a split group by any member session. */
export function setGroupNameForSession(
  sessionId: string,
  name: string | null,
): void {
  setGroupNameAtIndex(findLayoutIndexForSession(sessionId), name);
}

// Sessions assigned to a pane this app-session, with assign time. React
// Router v7 wraps navigate() in startTransition, so "assign then navigate"
// commits in TWO renders: an urgent one where the layout already holds the
// new session but the URL still points at the old chat. RunnerChat's
// vanished-session sweep runs on that intermediate commit and would evict
// the just-assigned session (not in directSessions yet, not in the
// pre-spawn recentRows, not the URL session). The sweep consults this
// freshness window to leave brand-new assignments alone; genuinely stale
// layout members (restored from a previous app run) are never fresh
// because the map is in-memory.
const FRESH_ASSIGN_TTL_MS = 15_000;
const freshAssignments = new Map<string, number>();

export function isFreshlyAssigned(sessionId: string): boolean {
  const at = freshAssignments.get(sessionId);
  if (at === undefined) return false;
  if (Date.now() - at >= FRESH_ASSIGN_TTL_MS) {
    freshAssignments.delete(sessionId);
    return false;
  }
  return true;
}

export function assignSessionToPane(paneId: string, sessionId: string): void {
  freshAssignments.set(sessionId, Date.now());
  const nextLayouts = layouts.map((layout, index) =>
    index === activeIndex
      ? assignSessionPure(layout, paneId, sessionId)
      : removeSessionPure(layout, sessionId),
  );
  setLayoutSet(nextLayouts, activeIndex);
}

export function removeSessionFromLayout(sessionId: string): void {
  setLayoutSet(
    layouts.map((layout) => removeSessionPure(layout, sessionId)),
    activeIndex,
  );
}

/** Reconcile a successful backend archive without writing the tab set back.
 *  session_archive already removed the session and deleted an empty tab in
 *  one transaction; persisting this local cleanup could recreate that row. */
export function removeArchivedSessionFromLayout(sessionId: string): void {
  const activeId = currentLayout().id;
  const next = layouts
    .map((layout) => removeSessionPure(layout, sessionId))
    .filter((layout) => visibleSessionIds(layout.root).length > 0);
  const preservedIndex = next.findIndex((layout) => layout.id === activeId);
  applyLayoutSet(
    next,
    preservedIndex >= 0
      ? preservedIndex
      : Math.min(activeIndex, Math.max(0, next.length - 1)),
  );
}

export function focusPane(paneId: string): void {
  const active = currentLayout();
  if (active.focusedPaneId === paneId) return;
  if (!findLeaf(active.root, paneId)) return;
  setCurrent({ ...active, focusedPaneId: paneId });
}

export function closePane(paneId: string): PaneLayout {
  setCurrent(closePanePure(currentLayout(), paneId));
  return currentLayout();
}

/** Record live gutter sizes without notifying local subscribers — the panel
 *  lib owns the visual truth during a drag. Fires on drag end, not per frame,
 *  and writes the active tab through to SQLite. */
export function recordSplitSizes(
  splitId: string,
  sizes: [number, number],
): void {
  const walk = (node: PaneNode): void => {
    if (node.kind === "leaf") return;
    if (node.id === splitId) {
      node.sizes = sizes;
      return;
    }
    walk(node.a);
    walk(node.b);
  };
  walk(currentLayout().root);
  const layout = currentLayout();
  if (layout.id) {
    const input = layoutInput(layout, activeIndex);
    enqueueWrite(() => api.tab.upsert(input), "tab resize persist");
  }
}
