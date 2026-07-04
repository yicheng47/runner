// Direct-chat split-view layout model (impl 0020, spec 34).
//
// A layout is a binary tree of splits with 1–3 leaf panes, built only from
// the six picker presets (1 · 2 side-by-side · 2 stacked · 1-big+2-stacked ·
// 3 columns · 3 rows). Each leaf maps to at most one direct-chat session —
// a session lives in exactly one pane (move-not-copy), which is what keeps
// the single-writer stdin invariant: RunnerChat mounts one RunnerTerminal
// per session, ever, and the layout only decides which of them are visible.
//
// State is a module-level store shared by RunnerChat and Sidebar via
// `useSyncExternalStore`. It is per window and sticky (key decision 6):
// the main window persists pane tabs to localStorage so a relaunch restores
// them, and navigating off the chat surface keeps them.

import { useSyncExternalStore } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";

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
  focusedSlot: number;
  name: string | null;
}

interface PersistedLayoutSet {
  version: 2;
  activeSlot: number;
  tabs: PersistedLayout[];
}

function toPersistedLayout(layout: PaneLayout): PersistedLayout {
  const all = leaves(layout.root);
  const sizes: Record<string, [number, number]> = {};
  const walk = (node: PaneNode): void => {
    if (node.kind === "leaf") return;
    sizes[node.id] = node.sizes;
    walk(node.a);
    walk(node.b);
  };
  walk(layout.root);
  return {
    preset: layout.preset,
    slots: all.map((l) => l.sessionId),
    sizes,
    focusedSlot: Math.max(
      0,
      all.findIndex((l) => l.id === layout.focusedPaneId),
    ),
    name: layout.name,
  };
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

// Persist for the main window only: localStorage is shared by every
// webview window, and secondary windows' labels (`window-<ulid>`) don't
// survive a relaunch — persisting theirs would only clobber the main
// window's pane tabs.
let persistToStorage = false;
try {
  persistToStorage = getCurrentWindow().label === "main";
} catch {
  // Dev browser preview — single "window", persist normally.
  persistToStorage = true;
}

// ---- module store -------------------------------------------------------

function singleLayout(sessionId: string | null = null): PaneLayout {
  return {
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

function serializeLayoutSet(
  layouts: PaneLayout[],
  activeIndex: number,
): string {
  return JSON.stringify({
    version: 2,
    activeSlot: activeIndex,
    tabs: layouts.map(toPersistedLayout),
  } satisfies PersistedLayoutSet);
}

function readPersistedLayoutSet(): {
  layouts: PaneLayout[];
  activeIndex: number;
} | null {
  if (!persistToStorage) return null;
  try {
    const raw = localStorage.getItem(STORAGE_LAYOUT);
    return raw ? deserializeLayoutSet(raw) : null;
  } catch {
    return null;
  }
}

function persistLayoutSet(): void {
  if (!persistToStorage) return;
  try {
    localStorage.setItem(
      STORAGE_LAYOUT,
      serializeLayoutSet(layouts, activeIndex),
    );
  } catch {
    // best-effort
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

const persisted = readPersistedLayoutSet();
let layouts: PaneLayout[] = persisted?.layouts ?? [singleLayout()];
let activeIndex = persisted?.activeIndex ?? 0;
const listeners = new Set<() => void>();

function currentLayout(): PaneLayout {
  return layouts[activeIndex] ?? layouts[0] ?? singleLayout();
}

function findLayoutIndexForSession(sessionId: string | null): number {
  if (!sessionId) return -1;
  return layouts.findIndex((layout) => leafForSession(layout.root, sessionId));
}

function setLayoutSet(
  nextLayouts: PaneLayout[],
  nextActiveIndex: number,
): void {
  const normalized = normalizeLayoutSet(nextLayouts, nextActiveIndex);
  layouts = normalized.layouts;
  activeIndex = normalized.activeIndex;
  persistLayoutSet();
  for (const l of listeners) l();
}

function setCurrent(next: PaneLayout): void {
  if (next === currentLayout()) return;
  const nextLayouts = [...layouts];
  nextLayouts[activeIndex] = next;
  setLayoutSet(nextLayouts, activeIndex);
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

export function activatePaneLayoutForSession(
  sessionId: string | null,
): PaneLayout {
  const index = findLayoutIndexForSession(sessionId);
  if (index >= 0 && index !== activeIndex) {
    setLayoutSet(layouts, index);
    return currentLayout();
  }
  return index >= 0 ? layouts[index] : currentLayout();
}

export function resetPaneLayoutsForTest(
  nextLayouts: PaneLayout[] = [singleLayout()],
  nextActiveIndex = 0,
): void {
  const normalized = normalizeLayoutSet(nextLayouts, nextActiveIndex);
  layouts = normalized.layouts;
  activeIndex = normalized.activeIndex;
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
    nextLayouts[existingIndex] = next;
    setLayoutSet(nextLayouts, existingIndex);
    return currentLayout();
  }
  const active = currentLayout();
  if (
    active.preset === "single" &&
    visibleSessionIds(active.root).length <= 1
  ) {
    setCurrent(next);
    return currentLayout();
  }
  setLayoutSet([...layouts, next], layouts.length);
  return currentLayout();
}

/** Name (or un-name, with null/blank) the split group. */
export function setGroupName(name: string | null): void {
  const trimmed = name?.trim() || null;
  const active = currentLayout();
  if (active.name === trimmed) return;
  setCurrent({ ...active, name: trimmed });
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

/** Record live gutter sizes without notifying subscribers — the panel lib
 *  owns the visual truth during a drag; this only keeps the model (and
 *  the persisted copy) current. Fires on drag end, not per frame. */
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
  persistLayoutSet();
}
