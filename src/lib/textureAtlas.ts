// What invalidates the WebGL texture atlas (impl 0037, #360).
//
// The atlas caches glyphs rasterized at specific device-pixel dimensions.
// Anything that changes those dimensions leaves every cached glyph wrong,
// and the renderer has no way to know — `Terminal.refresh()` only marks
// lines dirty, so stale rasters get redrawn faithfully. #360 exists
// because the list of invalidators lived inline in one event handler and
// was missing entries; it lives here now so it is one list, in one place,
// with one test.
//
// The invalidators are independent, not a hierarchy: page zoom moves no
// resolution and no wake, a monitor move fires no wake, and a GPU reset
// on wake moves no resolution. Focus is NOT one of them — it correlates
// with wake but fires on every alt-tab, and each clear costs a full
// re-rasterization on the next draw.
//
// ## Backing scale
//
// A `(resolution: Xdppx)` media query matches only while the display scale
// is exactly X, so the change event fires as the query STOPS matching and
// the listener has to be re-registered at the new scale to catch the next
// one. That is the whole reason this is not a one-shot `addEventListener`.
//
// Note the overlap, which is deliberate: xterm's own `ScreenDprMonitor`
// runs the same query shape and re-keys the char atlas by
// `devicePixelRatio`, so a plain scale change is already handled upstream.
// What it does not cover is a display re-initialization that drops GPU
// texture contents on the way to a scale it has already cached — the atlas
// config compares equal and the stale entry is reused. Clearing here is
// cheap at these frequencies and closes that case.
//
// Deliberately NOT a proxy for wake: page zoom never moves
// `devicePixelRatio` under WebKit (webkit#124862, see windowSettle.ts), and
// a GPU reset at an unchanged scale fires no resolution change at all.
// Those are separate invalidators with separate signals.

import {
  STORAGE_APP_ZOOM,
  STORAGE_TERMINAL_FONT_FAMILY,
  STORAGE_TERMINAL_FONT_SIZE,
} from "./settings";

/**
 * Settings whose change restales every cached glyph. Font size and family
 * change the rasterized cell outright; app zoom changes how many device
 * pixels that cell occupies, which the atlas keys on just as hard.
 *
 * Cursor style, scrollback, and theme are not here on purpose: none of
 * them changes a glyph's rendered dimensions.
 */
export function stalesTextureAtlas(key: string | null): boolean {
  return (
    key === STORAGE_TERMINAL_FONT_SIZE ||
    key === STORAGE_TERMINAL_FONT_FAMILY ||
    key === STORAGE_APP_ZOOM
  );
}

/** The `MediaQueryList` surface this uses, narrowed so tests can fake it. */
export interface ResolutionQuery {
  addEventListener(type: "change", listener: () => void): void;
  removeEventListener(type: "change", listener: () => void): void;
}

/** The `window` surface this uses, narrowed so tests can fake it. */
export interface BackingScaleWindow {
  readonly devicePixelRatio: number;
  matchMedia(query: string): ResolutionQuery;
}

export function resolutionQuery(dpr: number): string {
  return `(resolution: ${dpr}dppx)`;
}

/**
 * Call `onChange` with the new device pixel ratio each time the backing
 * scale changes. Returns a disposer.
 */
export function observeBackingScale(
  onChange: (dpr: number) => void,
  win: BackingScaleWindow = window,
): () => void {
  let query: ResolutionQuery | null = null;
  let disposed = false;

  const listener = () => {
    if (disposed) return;
    register();
    onChange(win.devicePixelRatio);
  };

  function register() {
    query?.removeEventListener("change", listener);
    query = win.matchMedia(resolutionQuery(win.devicePixelRatio));
    query.addEventListener("change", listener);
  }

  register();

  return () => {
    disposed = true;
    query?.removeEventListener("change", listener);
    query = null;
  };
}
