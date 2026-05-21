// Render-gate companion: returns true only after `active` has held
// true for `delayMs`. Used to suppress transitional "Starting…" /
// "Loading…" pills on the common fast-IPC path — the underlying
// render gate (metaLoaded, mission loading, etc.) still blocks
// content so we keep correctness; we only delay the *visible* pill
// so chat-to-chat and mission-to-mission nav doesn't flash cyan.
//
// Each fresh (active=true, resetKey) window gets its own
// monotonically increasing generation. The timer captures the gen
// at schedule time; the return value compares it against the
// *current* gen at render time. Keying on a per-window gen (not the
// raw resetKey value) matters for A→B→A: the same id can start a
// new window after active toggled off-and-on or after resetKey
// changed away and back, and we don't want a stale fire from the
// first A window to match the second A window's resetKey.
//
// Prev-value tracking lives in `useState` rather than `useRef`
// because reading a ref during render is forbidden by our React
// rules — state's documented "store info from prev renders"
// pattern (setState during render → React replays the render with
// new state) gives us the same behavior cleanly.
//
// First introduced in #179 for direct-chat navigation; lifted out
// of `RunnerChat.tsx` so `MissionWorkspace.tsx` can use the same
// gate for its "Starting mission…" pill.

import { useEffect, useState } from "react";

export function useDelayedFlag(
  active: boolean,
  delayMs: number,
  resetKey?: unknown,
): boolean {
  const [state, setState] = useState<{
    gen: number;
    prevActive: boolean;
    prevResetKey: unknown;
    // -1 sentinel for "no timer has fired" — gen starts at 0 and
    // only increments, so it can't collide.
    shownGen: number;
  }>({ gen: 0, prevActive: false, prevResetKey: undefined, shownGen: -1 });

  // Detect a window transition during this render. Bumping gen
  // synchronously is what makes a repeated resetKey value (A→B→A)
  // get a distinct window, so a stale fire from the first A window
  // can't match the second.
  let currentGen = state.gen;
  if (state.prevActive !== active || state.prevResetKey !== resetKey) {
    if (active && (!state.prevActive || state.prevResetKey !== resetKey)) {
      currentGen = state.gen + 1;
    }
    setState({
      gen: currentGen,
      prevActive: active,
      prevResetKey: resetKey,
      shownGen: state.shownGen,
    });
  }

  useEffect(() => {
    if (!active) return;
    const captured = currentGen;
    const t = window.setTimeout(
      () => setState((s) => ({ ...s, shownGen: captured })),
      delayMs,
    );
    return () => window.clearTimeout(t);
  }, [active, delayMs, currentGen]);

  return active && state.shownGen === currentGen;
}
