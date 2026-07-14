import { describe, expect, it } from "vitest";

import { KEYMAP, type KeymapScope } from "./keymap";

const SCOPES: readonly KeymapScope[] = ["global", "chat-split", "mission"];

describe("keymap registry", () => {
  it("has no duplicate ids", () => {
    const ids = KEYMAP.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("has no duplicate key chips within a scope", () => {
    const keys = KEYMAP.flatMap((entry) =>
      entry.keys.map((key) => `${entry.scope}:${key}`),
    );
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("every entry has a known scope, title, description, and keys", () => {
    for (const entry of KEYMAP) {
      expect(SCOPES).toContain(entry.scope);
      expect(entry.title.length).toBeGreaterThan(0);
      expect(entry.description.length).toBeGreaterThan(0);
      expect(entry.keys.length).toBeGreaterThan(0);
      for (const key of entry.keys) {
        expect(key.length).toBeGreaterThan(0);
      }
    }
  });
});
