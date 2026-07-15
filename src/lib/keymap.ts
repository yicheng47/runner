// Keymap registry + rebinding layer — feature #273 (v2 of #257).
//
// One entry per action (Codex-style rows: "Previous page" and "Next
// page" are separate entries, not one row with two chips). Every
// handler matches events through `eventMatchesShortcut`, so the
// effective binding — default or user override — is the single source
// of truth and handlers carry no key literals.
//
// Overrides persist in localStorage (`settings.keymapOverrides`) keyed
// by entry id: a stored combo replaces the default, an explicit `null`
// unbinds the action, an absent key means "use the default". Reads go
// straight to localStorage (tiny JSON, keydown-frequency is nothing),
// which keeps multiple windows coherent without cache invalidation.

export type KeymapScope = "global" | "chat-split" | "mission";

export interface KeyCombo {
  meta: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  /** KeyboardEvent.code — layout-independent physical key. */
  code: string;
  /** Display glyph for the key part when it beats the code-derived one
   *  (e.g. "+" for Shift+Equal captured on record). */
  label?: string;
  /** Match regardless of Shift. Only the zoom-in default uses this:
   *  "+" is Shift+"=" on US layouts, so ⌘= and ⌘⇧= must both zoom. */
  shiftOptional?: boolean;
}

export interface KeymapEntry {
  id: string;
  title: string;
  description: string;
  scope: KeymapScope;
  default: KeyCombo;
  /** Owned by an OS menu accelerator (impl 0018) — shown but not
   *  rebindable; native menu rebinding is a separate problem. */
  fixed?: boolean;
}

const combo = (
  code: string,
  mods: { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean },
  extra?: Pick<KeyCombo, "label" | "shiftOptional">,
): KeyCombo => ({
  meta: mods.meta ?? false,
  ctrl: mods.ctrl ?? false,
  alt: mods.alt ?? false,
  shift: mods.shift ?? false,
  code,
  ...extra,
});

export const KEYMAP: readonly KeymapEntry[] = [
  {
    id: "new-window",
    title: "New window",
    description: "Open another Runner window.",
    scope: "global",
    default: combo("KeyN", { meta: true }),
    fixed: true,
  },
  {
    id: "new-chat",
    title: "New chat",
    description: "Start a chat in a new tab.",
    scope: "global",
    default: combo("KeyT", { meta: true }),
  },
  {
    id: "command-palette",
    title: "Command palette",
    description: "Search missions, chats, runners, and crews.",
    scope: "global",
    default: combo("KeyK", { meta: true }),
  },
  {
    id: "toggle-sidebar",
    title: "Toggle sidebar",
    description: "Collapse or expand the app sidebar.",
    scope: "global",
    default: combo("KeyS", { meta: true }),
  },
  {
    id: "open-settings",
    title: "Open settings",
    description: "Open this settings page.",
    scope: "global",
    default: combo("Comma", { meta: true }),
  },
  {
    id: "page-previous",
    title: "Previous page",
    description: "Step back through recently viewed missions and chats.",
    scope: "global",
    default: combo("BracketLeft", { meta: true, shift: true }),
  },
  {
    id: "page-next",
    title: "Next page",
    description: "Step forward through recently viewed missions and chats.",
    scope: "global",
    default: combo("BracketRight", { meta: true, shift: true }),
  },
  {
    id: "zoom-in",
    title: "Zoom in",
    description: "Scale the whole app up.",
    scope: "global",
    default: combo("Equal", { meta: true }, { label: "+", shiftOptional: true }),
  },
  {
    id: "zoom-out",
    title: "Zoom out",
    description: "Scale the whole app down.",
    scope: "global",
    default: combo("Minus", { meta: true }),
  },
  {
    id: "zoom-reset",
    title: "Reset zoom",
    description: "Return the app to 100%.",
    scope: "global",
    default: combo("Digit0", { meta: true }),
  },
  {
    id: "pane-previous",
    title: "Previous chat pane",
    description: "Focus the previous pane while a chat is split.",
    scope: "chat-split",
    default: combo("BracketLeft", { meta: true }),
  },
  {
    id: "pane-next",
    title: "Next chat pane",
    description: "Focus the next pane while a chat is split.",
    scope: "chat-split",
    default: combo("BracketRight", { meta: true }),
  },
  {
    id: "close-pane",
    title: "Close pane",
    description: "Collapse the focused pane while a chat is split.",
    scope: "chat-split",
    default: combo("KeyW", { meta: true }),
  },
  {
    id: "mission-tab-previous",
    title: "Previous mission tab",
    description: "Cycle back through the feed and open runner tabs.",
    scope: "mission",
    default: combo("BracketLeft", { meta: true }),
  },
  {
    id: "mission-tab-next",
    title: "Next mission tab",
    description: "Cycle forward through the feed and open runner tabs.",
    scope: "mission",
    default: combo("BracketRight", { meta: true }),
  },
];

const byId = new Map(KEYMAP.map((entry) => [entry.id, entry]));

export function keymapEntry(id: string): KeymapEntry {
  const entry = byId.get(id);
  if (!entry) throw new Error(`unknown keymap entry: ${id}`);
  return entry;
}

// ---------------------------------------------------------------------------
// Override store

const STORAGE_KEYMAP_OVERRIDES = "settings.keymapOverrides";

/** Fired on window whenever an override changes, so open panes re-read. */
export const KEYMAP_CHANGED_EVENT = "runner:keymap-changed";

/** id → replacement combo, or null for "unbound". Absent id = default. */
export type KeymapOverrides = Record<string, KeyCombo | null>;

function isKeyCombo(value: unknown): value is KeyCombo {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.code === "string" &&
    typeof v.meta === "boolean" &&
    typeof v.ctrl === "boolean" &&
    typeof v.alt === "boolean" &&
    typeof v.shift === "boolean"
  );
}

export function readKeymapOverrides(): KeymapOverrides {
  if (typeof localStorage === "undefined") return {};
  const raw = localStorage.getItem(STORAGE_KEYMAP_OVERRIDES);
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const overrides: KeymapOverrides = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (!byId.has(id)) continue;
      if (value === null || isKeyCombo(value)) overrides[id] = value;
    }
    return overrides;
  } catch {
    return {};
  }
}

function writeKeymapOverrides(overrides: KeymapOverrides) {
  if (typeof localStorage === "undefined") return;
  if (Object.keys(overrides).length === 0) {
    localStorage.removeItem(STORAGE_KEYMAP_OVERRIDES);
  } else {
    localStorage.setItem(STORAGE_KEYMAP_OVERRIDES, JSON.stringify(overrides));
  }
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(KEYMAP_CHANGED_EVENT));
  }
}

/** Rebind (combo) or unbind (null) one action. */
export function setKeymapOverride(id: string, value: KeyCombo | null) {
  keymapEntry(id);
  writeKeymapOverrides({ ...readKeymapOverrides(), [id]: value });
}

/** Restore one action to its default binding, or return the action that
 *  currently owns that combo when restoring would create a conflict. */
export function clearKeymapOverride(id: string): KeymapEntry | null {
  const overrides = readKeymapOverrides();
  if (!(id in overrides)) return null;
  const entry = keymapEntry(id);
  const conflict = findKeymapConflict(entry.default, id);
  if (conflict) return conflict;
  delete overrides[id];
  writeKeymapOverrides(overrides);
  return null;
}

export function resetKeymapOverrides() {
  writeKeymapOverrides({});
}

/** The binding handlers should honor right now: override ?? default. */
export function effectiveBinding(id: string): KeyCombo | null {
  const entry = keymapEntry(id);
  if (entry.fixed) return entry.default;
  const overrides = readKeymapOverrides();
  return id in overrides ? overrides[id] : entry.default;
}

// ---------------------------------------------------------------------------
// Event matching

/** Structural subset of KeyboardEvent so pure-logic tests run in node. */
export interface KeyEventLike {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  code: string;
}

let matchingSuspended = false;

/** While the Settings pane records a new combo, every handler's
 *  matcher goes dark so half-typed bindings don't trigger actions. */
export function suspendShortcutMatching(suspended: boolean) {
  matchingSuspended = suspended;
}

function comboMatchesEvent(combo: KeyCombo, e: KeyEventLike): boolean {
  return (
    e.code === combo.code &&
    e.metaKey === combo.meta &&
    e.ctrlKey === combo.ctrl &&
    e.altKey === combo.alt &&
    (combo.shiftOptional ? true : e.shiftKey === combo.shift)
  );
}

export function eventMatchesShortcut(e: KeyEventLike, id: string): boolean {
  if (matchingSuspended) return false;
  const binding = effectiveBinding(id);
  return binding !== null && comboMatchesEvent(binding, e);
}

// ---------------------------------------------------------------------------
// Conflicts

function scopesOverlap(a: KeymapScope, b: KeymapScope): boolean {
  return a === b || a === "global" || b === "global";
}

function combosCollide(a: KeyCombo, b: KeyCombo): boolean {
  return (
    a.code === b.code &&
    a.meta === b.meta &&
    a.ctrl === b.ctrl &&
    a.alt === b.alt &&
    (a.shiftOptional || b.shiftOptional || a.shift === b.shift)
  );
}

/** First entry whose effective binding would fire on the same combo in
 *  an overlapping scope, or null. Fixed entries count — binding ⌘N
 *  elsewhere would double-fire with the OS menu accelerator. */
export function findKeymapConflict(
  candidate: KeyCombo,
  forId: string,
): KeymapEntry | null {
  const target = keymapEntry(forId);
  for (const entry of KEYMAP) {
    if (entry.id === forId) continue;
    if (!scopesOverlap(entry.scope, target.scope)) continue;
    const binding = effectiveBinding(entry.id);
    if (binding && combosCollide(binding, candidate)) return entry;
  }
  return null;
}

// ---------------------------------------------------------------------------
// Capture + display

const MODIFIER_CODES = /^(Meta|Control|Alt|Shift)(Left|Right)?$/;

/** Capture a KeyboardEvent as a bindable combo, or null when it isn't
 *  one (modifier-only, or a bare key that would hijack typing — plain
 *  and shift-only keys are allowed for F-keys only). */
export function comboFromEvent(e: KeyEventLike & { key: string }): KeyCombo | null {
  if (MODIFIER_CODES.test(e.code) || e.code === "") return null;
  const hasRealModifier = e.metaKey || e.ctrlKey || e.altKey;
  if (!hasRealModifier && !/^F\d{1,2}$/.test(e.code)) return null;
  const printable =
    e.key.length === 1 && e.key !== " " ? e.key.toUpperCase() : undefined;
  return {
    meta: e.metaKey,
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    code: e.code,
    ...(printable && printable !== defaultKeyLabel(e.code)
      ? { label: printable }
      : {}),
  };
}

const CODE_LABELS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Enter: "↩",
  Tab: "⇥",
  Space: "Space",
  Backspace: "⌫",
  Delete: "⌦",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Home: "↖",
  End: "↘",
  PageUp: "⇞",
  PageDown: "⇟",
};

function defaultKeyLabel(code: string): string {
  const fromMap = CODE_LABELS[code];
  if (fromMap) return fromMap;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) return digit[1];
  return code;
}

/** "⌃⌥⇧⌘X" — macOS modifier order, then the key glyph. */
export function formatCombo(combo: KeyCombo): string {
  return (
    (combo.ctrl ? "⌃" : "") +
    (combo.alt ? "⌥" : "") +
    (combo.shift && !combo.shiftOptional ? "⇧" : "") +
    (combo.meta ? "⌘" : "") +
    (combo.label ?? defaultKeyLabel(combo.code))
  );
}
