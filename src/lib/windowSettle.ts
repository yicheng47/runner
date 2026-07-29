// Window-restore settle gate for launch auto-resume (impl 0038, decision 1).
//
// `window_state::restore` (impl 0027) applies the saved frame during Tauri
// `setup`, before the webview evaluates any script, and `app_ready` reveals
// the window on top of that. The native frame is therefore final by the time
// the frontend runs — but the webview's own layout catches up asynchronously,
// so every geometry read the auto-resume queue makes is wrong until the DOM
// viewport agrees with that frame. The queue waits for the agreement, not for
// a timer: the native inner size is the authority and the viewport is the
// observation.
//
// Both sides are normalized to LOGICAL POINTS before comparing, and the page
// zoom comes from the stored setting rather than from `devicePixelRatio`.
// WebKit does not fold page zoom into `devicePixelRatio` (webkit#124862) — it
// reports the display scale and lets `innerWidth` absorb the zoom — and wry
// implements `setZoom` as `WKWebView.setPageZoom`. Inferring zoom from DPR
// would therefore never converge at any non-100% app zoom, and every launch
// would burn the full ceiling.
//
// Width only: it is the dimension that decides wrapping (the whole point of
// #363), and both axes settle in the same layout pass anyway. Comparing both
// would only add a second way to never converge.

import { getCurrentWindow } from "@tauri-apps/api/window";

import { readAppZoom } from "./settings";

/** Hard ceiling on the wait. Reaching it degrades to resuming with whatever
 *  geometry is readable — never to hanging the queue. */
export const WINDOW_SETTLE_CEILING_MS = 1500;

/** `innerWidth` is an integer CSS pixel count, so the logical-point
 *  comparison carries up to one zoom step of rounding. */
export const WINDOW_SETTLE_TOLERANCE_PX = 4;

const WINDOW_SETTLE_POLL_MS = 32;

export type WindowSettleOutcome =
  /** Viewport agrees with the native frame. */
  | "settled"
  /** Ceiling hit first — proceed anyway. */
  | "timeout"
  /** No native window to compare against (browser preview). */
  | "unavailable";

/** The raw environment readings the gate normalizes. Split out from
 *  `WindowSettleDeps` so the unit conversion itself is testable. */
export interface WindowSettleEnv {
  /** Native inner width, in physical pixels. */
  nativeInnerWidthPx: () => Promise<number>;
  /** Display scale — physical pixels per logical point. */
  nativeScaleFactor: () => Promise<number>;
  /** Webview viewport width, in CSS pixels. */
  viewportCssWidth: () => number;
  /** Page zoom applied to this webview. */
  appZoom: () => number;
}

export interface WindowSettleDeps {
  /** Native inner width in logical points, or null when unreadable. */
  nativeLogicalWidth: () => Promise<number | null>;
  /** Webview viewport width in logical points. */
  viewportLogicalWidth: () => number;
  /** One wait step between comparisons. */
  step: () => Promise<void>;
  now: () => number;
  ceilingMs?: number;
}

/** Normalize environment readings into the logical points the gate compares. */
export function windowSettleDeps(env: WindowSettleEnv): WindowSettleDeps {
  return {
    nativeLogicalWidth: async () => {
      const [physicalWidth, scaleFactor] = await Promise.all([
        env.nativeInnerWidthPx(),
        env.nativeScaleFactor(),
      ]);
      return scaleFactor > 0 ? physicalWidth / scaleFactor : null;
    },
    viewportLogicalWidth: () => env.viewportCssWidth() * env.appZoom(),
    // rAF is the responsive signal, but macOS pauses it for occluded
    // windows — the timer keeps the ceiling enforceable either way.
    step: () =>
      new Promise<void>((resolve) => {
        window.setTimeout(resolve, WINDOW_SETTLE_POLL_MS);
        window.requestAnimationFrame(() => resolve());
      }),
    now: () => Date.now(),
  };
}

function defaultDeps(): WindowSettleDeps {
  return windowSettleDeps({
    nativeInnerWidthPx: async () => (await getCurrentWindow().innerSize()).width,
    nativeScaleFactor: () => getCurrentWindow().scaleFactor(),
    viewportCssWidth: () => window.innerWidth,
    appZoom: readAppZoom,
  });
}

/**
 * Resolve once the webview viewport reflects the restored window frame.
 * Never rejects and never waits longer than the ceiling.
 */
export async function awaitWindowGeometrySettle(
  deps: WindowSettleDeps = defaultDeps(),
): Promise<WindowSettleOutcome> {
  let nativeWidth: number | null;
  try {
    nativeWidth = await deps.nativeLogicalWidth();
  } catch {
    return "unavailable";
  }
  if (nativeWidth === null || !(nativeWidth > 0)) return "unavailable";

  const deadline = deps.now() + (deps.ceilingMs ?? WINDOW_SETTLE_CEILING_MS);
  for (;;) {
    const viewportWidth = deps.viewportLogicalWidth();
    if (
      viewportWidth > 0 &&
      Math.abs(viewportWidth - nativeWidth) <= WINDOW_SETTLE_TOLERANCE_PX
    ) {
      return "settled";
    }
    if (deps.now() >= deadline) return "timeout";
    await deps.step();
  }
}
