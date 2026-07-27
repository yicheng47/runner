// Two contracts (impl 0037, phase 2).
//
// The storage-key list: exactly the settings that change a glyph's
// rendered dimensions stale the atlas, and nothing else does. #360's whole
// shape was an invalidator missing from this list.
//
// The backing-scale watcher: it re-registers at the new scale on every
// change so the second display move is caught as reliably as the first, it
// reports the new ratio, and it goes quiet after disposal.

import { describe, expect, it, vi } from "vitest";

import {
  STORAGE_APP_ZOOM,
  STORAGE_TERMINAL_CURSOR_STYLE,
  STORAGE_TERMINAL_FONT_FAMILY,
  STORAGE_TERMINAL_FONT_SIZE,
  STORAGE_TERMINAL_SCROLLBACK,
  STORAGE_TERMINAL_THEME,
} from "./settings";
import {
  observeBackingScale,
  resolutionQuery,
  stalesTextureAtlas,
  type BackingScaleWindow,
  type ResolutionQuery,
} from "./textureAtlas";

/** A `window` whose scale is settable, tracking every query it hands out. */
function fakeWindow(initialDpr: number) {
  const queries: { query: string; listeners: Set<() => void> }[] = [];
  let dpr = initialDpr;

  const win: BackingScaleWindow = {
    get devicePixelRatio() {
      return dpr;
    },
    matchMedia(query: string): ResolutionQuery {
      const listeners = new Set<() => void>();
      queries.push({ query, listeners });
      return {
        addEventListener: (_type, listener) => {
          listeners.add(listener);
        },
        removeEventListener: (_type, listener) => {
          listeners.delete(listener);
        },
      };
    },
  };

  return {
    win,
    queries,
    /** Move the display scale and fire the outgoing query's change event. */
    setDpr(next: number) {
      const active = queries[queries.length - 1];
      dpr = next;
      for (const listener of [...active.listeners]) listener();
    },
    liveListenerCount() {
      return queries.reduce((n, q) => n + q.listeners.size, 0);
    },
    latestQuery() {
      return queries[queries.length - 1].query;
    },
  };
}

describe("stalesTextureAtlas", () => {
  it("stales on app zoom — the invalidator #360 was missing", () => {
    expect(stalesTextureAtlas(STORAGE_APP_ZOOM)).toBe(true);
  });

  it("stales on the font settings that change cell metrics", () => {
    expect(stalesTextureAtlas(STORAGE_TERMINAL_FONT_SIZE)).toBe(true);
    expect(stalesTextureAtlas(STORAGE_TERMINAL_FONT_FAMILY)).toBe(true);
  });

  it("leaves the atlas alone for settings that don't resize a glyph", () => {
    expect(stalesTextureAtlas(STORAGE_TERMINAL_CURSOR_STYLE)).toBe(false);
    expect(stalesTextureAtlas(STORAGE_TERMINAL_SCROLLBACK)).toBe(false);
    expect(stalesTextureAtlas(STORAGE_TERMINAL_THEME)).toBe(false);
  });

  it("ignores unrelated and null storage keys", () => {
    expect(stalesTextureAtlas("settings.somethingElse")).toBe(false);
    expect(stalesTextureAtlas(null)).toBe(false);
  });
});

describe("observeBackingScale", () => {
  it("registers a resolution query at the current scale", () => {
    const env = fakeWindow(2);
    observeBackingScale(vi.fn(), env.win);

    expect(env.queries).toHaveLength(1);
    expect(env.queries[0].query).toBe(resolutionQuery(2));
  });

  it("re-registers at the new scale so consecutive changes both fire", () => {
    const env = fakeWindow(2);
    const onChange = vi.fn();
    observeBackingScale(onChange, env.win);

    env.setDpr(1);
    expect(onChange).toHaveBeenCalledWith(1);
    expect(env.latestQuery()).toBe(resolutionQuery(1));

    env.setDpr(2);
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenLastCalledWith(2);
    expect(env.latestQuery()).toBe(resolutionQuery(2));
  });

  it("leaves exactly one live listener behind after re-registering", () => {
    const env = fakeWindow(2);
    observeBackingScale(vi.fn(), env.win);

    env.setDpr(1);
    env.setDpr(3);

    expect(env.liveListenerCount()).toBe(1);
  });

  it("does not fire on registration", () => {
    const env = fakeWindow(2);
    const onChange = vi.fn();
    observeBackingScale(onChange, env.win);

    expect(onChange).not.toHaveBeenCalled();
  });

  it("stops reporting and unregisters once disposed", () => {
    const env = fakeWindow(2);
    const onChange = vi.fn();
    const dispose = observeBackingScale(onChange, env.win);

    dispose();
    expect(env.liveListenerCount()).toBe(0);

    env.setDpr(1);
    expect(onChange).not.toHaveBeenCalled();
  });
});
