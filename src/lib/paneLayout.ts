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
// the main window persists it to localStorage so a relaunch restores the
// same pane grouping, and navigating off the chat surface keeps it.

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

export function serializeLayout(layout: PaneLayout): string {
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
    focusedSlot: Math.max(
      0,
      all.findIndex((l) => l.id === layout.focusedPaneId),
    ),
    name: layout.name,
  };
  return JSON.stringify(persisted);
}

/** Rebuild a layout from a persisted payload; null on any malformed or
 *  unrecognized input (callers fall back to a fresh single pane). */
export function deserializeLayout(raw: string): PaneLayout | null {
  try {
    const p = JSON.parse(raw) as Partial<PersistedLayout> | null;
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
  } catch {
    return null;
  }
}

// Persist for the main window only: localStorage is shared by every
// webview window, and secondary windows' labels (`window-<ulid>`) don't
// survive a relaunch — persisting theirs would only clobber the main
// window's layout.
let persistToStorage = false;
try {
  persistToStorage = getCurrentWindow().label === "main";
} catch {
  // Dev browser preview — single "window", persist normally.
  persistToStorage = true;
}

function readPersistedLayout(): PaneLayout | null {
  if (!persistToStorage) return null;
  try {
    const raw = localStorage.getItem(STORAGE_LAYOUT);
    return raw ? deserializeLayout(raw) : null;
  } catch {
    return null;
  }
}

function persistLayout(layout: PaneLayout): void {
  if (!persistToStorage) return;
  try {
    localStorage.setItem(STORAGE_LAYOUT, serializeLayout(layout));
  } catch {
    // best-effort
  }
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

let current: PaneLayout = readPersistedLayout() ?? singleLayout();
const listeners = new Set<() => void>();

function setCurrent(next: PaneLayout): void {
  if (next === current) return;
  current = next;
  persistLayout(current);
  for (const l of listeners) l();
}

export function getPaneLayout(): PaneLayout {
  return current;
}

export function subscribePaneLayout(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function usePaneLayout(): PaneLayout {
  return useSyncExternalStore(subscribePaneLayout, getPaneLayout);
}

export function applyPreset(
  kind: PresetKind,
  focusedSessionId: string | null,
  currentVisible: string[],
  name: string | null = null,
): PaneLayout {
  setCurrent(applyPresetPure(kind, focusedSessionId, currentVisible, name));
  return current;
}

/** Name (or un-name, with null/blank) the split group. */
export function setGroupName(name: string | null): void {
  const trimmed = name?.trim() || null;
  if (current.name === trimmed) return;
  setCurrent({ ...current, name: trimmed });
}

export function assignSessionToPane(paneId: string, sessionId: string): void {
  setCurrent(assignSessionPure(current, paneId, sessionId));
}

export function removeSessionFromLayout(sessionId: string): void {
  setCurrent(removeSessionPure(current, sessionId));
}

export function focusPane(paneId: string): void {
  if (current.focusedPaneId === paneId) return;
  if (!findLeaf(current.root, paneId)) return;
  setCurrent({ ...current, focusedPaneId: paneId });
}

export function closePane(paneId: string): PaneLayout {
  setCurrent(closePanePure(current, paneId));
  return current;
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
  walk(current.root);
  persistLayout(current);
}
