import type { Terminal } from "@xterm/xterm";

/**
 * True when xterm holds no visible content at all — no glyphs in the
 * viewport and no scrollback. This is the signature of a replayed
 * empty output ring (`t.reset()` + zero bytes, e.g. a session stopped
 * before the last app launch): only the agent's own SIGWINCH-driven
 * repaint can paint such a pane, so a refresh on a running session
 * must run the resize dance even when the transitional latch or the
 * last-pushed dedupe would normally suppress it (#312).
 */
export function terminalGridIsBlank(t: Terminal): boolean {
  const buf = t.buffer.active;
  // Scrollback implies painted content; only scan viewport-sized buffers.
  if (buf.length > t.rows) return false;
  for (let y = 0; y < buf.length; y += 1) {
    const line = buf.getLine(y);
    if (line && line.translateToString(true).trim().length > 0) {
      return false;
    }
  }
  return true;
}

/**
 * Should a plain backend-size push escalate to the resize dance?
 *
 * - `"dance"`: the grid is blank and settled — nothing but the agent's
 *   SIGWINCH repaint can paint it, so force the dance.
 * - `"defer"`: the grid reads blank but live bytes are queued in
 *   xterm's write buffer and not yet parsed — `write()` is async, so a
 *   synchronous buffer read can't see them. The queued bytes may BE
 *   the repaint; dancing now would double-SIGWINCH a pane that is
 *   about to hold content, exactly what the transitional latch guards
 *   against. The caller must re-check after the queue flushes.
 * - `"none"`: the grid holds content — keep the plain-push path and
 *   the latch's guarantee untouched.
 */
export function blankDanceDecision(
  t: Terminal,
  pendingLiveWrites: number,
): "dance" | "defer" | "none" {
  if (!terminalGridIsBlank(t)) return "none";
  return pendingLiveWrites > 0 ? "defer" : "dance";
}

/**
 * Per-terminal bookkeeping behind the `"defer"` decision: counts live
 * writes queued in xterm's async parser and coalesces any number of
 * deferred blank rechecks into exactly ONE when the queue drains.
 *
 * Coalescing is the point — overlapping refresh passes (an ordinary
 * wake invokes its refit twice; activation and the ResizeObserver
 * retry can overlap too) may each observe blank+pending against the
 * SAME in-flight write. If every observation queued its own recheck,
 * a flush that parsed only control bytes (grid still blank) would run
 * them all and each would dance — duplicate SIGWINCHes, the exact
 * risk the defer exists to prevent.
 */
export function createBlankRecheckGate() {
  let pendingWrites = 0;
  let recheckPending = false;
  return {
    pendingWrites() {
      return pendingWrites;
    },
    beginWrite() {
      pendingWrites += 1;
    },
    /**
     * Returns true exactly when this write drained the queue AND a
     * recheck was requested since the last flush — the caller runs
     * one recheck.
     */
    endWrite() {
      pendingWrites -= 1;
      if (pendingWrites > 0 || !recheckPending) return false;
      recheckPending = false;
      return true;
    },
    requestRecheck() {
      recheckPending = true;
    },
    /** Drop a stale request (session swap / listener teardown). */
    cancelRecheck() {
      recheckPending = false;
    },
  };
}
