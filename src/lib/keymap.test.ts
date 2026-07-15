import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  KEYMAP,
  clearKeymapOverride,
  comboFromEvent,
  effectiveBinding,
  eventMatchesShortcut,
  findKeymapConflict,
  formatCombo,
  keymapEntry,
  readKeymapOverrides,
  resetKeymapOverrides,
  setKeymapOverride,
  suspendShortcutMatching,
  type KeyCombo,
  type KeyEventLike,
  type KeymapScope,
} from "./keymap";

const SCOPES: readonly KeymapScope[] = ["global", "chat-split", "mission"];

class MemoryStorage implements Storage {
  private items = new Map<string, string>();

  get length() {
    return this.items.size;
  }

  clear() {
    this.items.clear();
  }

  getItem(key: string) {
    return this.items.get(key) ?? null;
  }

  key(index: number) {
    return [...this.items.keys()][index] ?? null;
  }

  removeItem(key: string) {
    this.items.delete(key);
  }

  setItem(key: string, value: string) {
    this.items.set(key, value);
  }
}

const keyCombo = (
  code: string,
  modifiers: Partial<Omit<KeyCombo, "code">> = {},
): KeyCombo => ({
  code,
  meta: false,
  ctrl: false,
  alt: false,
  shift: false,
  ...modifiers,
});

const keyEvent = (
  code: string,
  modifiers: Partial<Omit<KeyEventLike, "code">> = {},
): KeyEventLike => ({
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...modifiers,
});

beforeEach(() => {
  Object.defineProperty(globalThis, "localStorage", {
    value: new MemoryStorage(),
    configurable: true,
  });
});

afterEach(() => {
  suspendShortcutMatching(false);
  delete (globalThis as { localStorage?: Storage }).localStorage;
});

describe("keymap registry", () => {
  it("has one complete entry per action with unique ids", () => {
    const ids = KEYMAP.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual([
      "new-window",
      "new-chat",
      "command-palette",
      "toggle-sidebar",
      "open-settings",
      "page-previous",
      "page-next",
      "zoom-in",
      "zoom-out",
      "zoom-reset",
      "pane-previous",
      "pane-next",
      "close-pane",
      "mission-tab-previous",
      "mission-tab-next",
    ]);
    for (const entry of KEYMAP) {
      expect(SCOPES).toContain(entry.scope);
      expect(entry.title.length).toBeGreaterThan(0);
      expect(entry.description.length).toBeGreaterThan(0);
      expect(entry.default.code.length).toBeGreaterThan(0);
      expect(formatCombo(entry.default).length).toBeGreaterThan(0);
    }
  });

  it("has no conflicting defaults in overlapping scopes", () => {
    for (const entry of KEYMAP) {
      expect(findKeymapConflict(entry.default, entry.id)).toBeNull();
    }
  });

  it("marks only the native new-window binding as fixed", () => {
    expect(KEYMAP.filter((entry) => entry.fixed).map((entry) => entry.id)).toEqual([
      "new-window",
    ]);
    expect(() => keymapEntry("missing-action")).toThrow(
      "unknown keymap entry: missing-action",
    );
  });
});

describe("keymap overrides", () => {
  it("round-trips rebound, unbound, and cleared entries", () => {
    const rebound = keyCombo("KeyP", { meta: true, shift: true });
    setKeymapOverride("command-palette", rebound);
    expect(readKeymapOverrides()).toEqual({ "command-palette": rebound });

    setKeymapOverride("command-palette", null);
    expect(readKeymapOverrides()).toEqual({ "command-palette": null });

    clearKeymapOverride("command-palette");
    expect(readKeymapOverrides()).toEqual({});
  });

  it("resets all overrides at once", () => {
    setKeymapOverride("command-palette", keyCombo("KeyP", { meta: true }));
    setKeymapOverride("new-chat", null);
    resetKeymapOverrides();
    expect(readKeymapOverrides()).toEqual({});
    expect(localStorage.getItem("settings.keymapOverrides")).toBeNull();
  });

  it("blocks restoring a default claimed by another action", () => {
    const commandPaletteDefault = keymapEntry("command-palette").default;
    setKeymapOverride("command-palette", null);
    setKeymapOverride("new-chat", commandPaletteDefault);

    expect(clearKeymapOverride("command-palette")?.id).toBe("new-chat");
    expect(effectiveBinding("command-palette")).toBeNull();
    expect(effectiveBinding("new-chat")).toEqual(commandPaletteDefault);
  });

  it("resolves defaults, rebound entries, unbound entries, and fixed entries", () => {
    expect(effectiveBinding("command-palette")).toEqual(
      keymapEntry("command-palette").default,
    );

    const rebound = keyCombo("KeyP", { ctrl: true });
    setKeymapOverride("command-palette", rebound);
    expect(effectiveBinding("command-palette")).toEqual(rebound);

    setKeymapOverride("command-palette", null);
    expect(effectiveBinding("command-palette")).toBeNull();

    setKeymapOverride("new-window", null);
    expect(effectiveBinding("new-window")).toEqual(
      keymapEntry("new-window").default,
    );
  });
});

describe("keymap conflicts", () => {
  it("allows the same combo in the chat-split and mission scopes", () => {
    expect(keymapEntry("pane-previous").default).toEqual(
      keymapEntry("mission-tab-previous").default,
    );
    expect(
      findKeymapConflict(
        keymapEntry("mission-tab-previous").default,
        "pane-previous",
      ),
    ).toBeNull();
  });

  it("detects global bindings against scoped actions", () => {
    const candidate = keymapEntry("pane-previous").default;
    setKeymapOverride("command-palette", candidate);
    expect(findKeymapConflict(candidate, "pane-previous")?.id).toBe(
      "command-palette",
    );
  });

  it("includes fixed global bindings in conflict detection", () => {
    expect(
      findKeymapConflict(keymapEntry("new-window").default, "pane-next")?.id,
    ).toBe("new-window");
  });
});

describe("eventMatchesShortcut", () => {
  it("requires the code and every modifier to match exactly", () => {
    expect(
      eventMatchesShortcut(keyEvent("KeyK", { metaKey: true }), "command-palette"),
    ).toBe(true);
    expect(
      eventMatchesShortcut(
        keyEvent("KeyK", { metaKey: true, shiftKey: true }),
        "command-palette",
      ),
    ).toBe(false);
    expect(
      eventMatchesShortcut(
        keyEvent("KeyK", { metaKey: true, ctrlKey: true }),
        "command-palette",
      ),
    ).toBe(false);
    expect(eventMatchesShortcut(keyEvent("KeyK"), "command-palette")).toBe(
      false,
    );
    expect(
      eventMatchesShortcut(keyEvent("KeyP", { metaKey: true }), "command-palette"),
    ).toBe(false);
  });

  it("allows either shift state for the zoom-in default only", () => {
    expect(
      eventMatchesShortcut(keyEvent("Equal", { metaKey: true }), "zoom-in"),
    ).toBe(true);
    expect(
      eventMatchesShortcut(
        keyEvent("Equal", { metaKey: true, shiftKey: true }),
        "zoom-in",
      ),
    ).toBe(true);
    expect(
      eventMatchesShortcut(
        keyEvent("Minus", { metaKey: true, shiftKey: true }),
        "zoom-out",
      ),
    ).toBe(false);
  });

  it("does not match while shortcut handling is suspended", () => {
    const event = keyEvent("KeyK", { metaKey: true });
    suspendShortcutMatching(true);
    expect(eventMatchesShortcut(event, "command-palette")).toBe(false);
    suspendShortcutMatching(false);
    expect(eventMatchesShortcut(event, "command-palette")).toBe(true);
  });
});

describe("comboFromEvent", () => {
  it("rejects modifier-only and bare non-function keys", () => {
    expect(
      comboFromEvent({
        ...keyEvent("MetaLeft", { metaKey: true }),
        key: "Meta",
      }),
    ).toBeNull();
    expect(comboFromEvent({ ...keyEvent("KeyA"), key: "a" })).toBeNull();
    expect(
      comboFromEvent({
        ...keyEvent("KeyA", { shiftKey: true }),
        key: "A",
      }),
    ).toBeNull();
  });

  it("captures modified keys and bare function keys", () => {
    expect(
      comboFromEvent({
        ...keyEvent("KeyP", { ctrlKey: true, shiftKey: true }),
        key: "P",
      }),
    ).toEqual(keyCombo("KeyP", { ctrl: true, shift: true }));
    expect(comboFromEvent({ ...keyEvent("F5"), key: "F5" })).toEqual(
      keyCombo("F5"),
    );
  });
});
