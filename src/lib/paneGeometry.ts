// Geometry sync for ChatPaneGroup's flat terminal stack (impl 0020,
// decision 4). Closure over plain Maps instead of refs so the
// callback-ref factories can be called during render (react-hooks/refs
// forbids `ref.current` there); the returned callbacks are stable per
// key so React doesn't detach/reattach them per commit.

import { logResizeGate } from "./frontendLog";
import { findLeaf, type PaneNode } from "./paneLayout";

export function createPaneGeometry() {
  let container: HTMLDivElement | null = null;
  let ro: ResizeObserver | null = null;
  let root: PaneNode | null = null;
  const paneBodies = new Map<string, HTMLDivElement>();
  const termWraps = new Map<string, HTMLDivElement>();
  // Sessions whose wrapper has been positioned from a real pane-body
  // rect at least once. Until then the wrapper is held at 0×0 (see
  // termWrapRefFor): an unplaced absolute wrapper otherwise shrink-wraps
  // xterm's default 80×24 canvas into a measurable ~78×23 box, and that
  // garbage fit got pushed to the PTY and purged the ring (#373 defect
  // 2). Zero size keeps the pane unmeasurable — the same state as a
  // display:none pane — so RunnerTerminal's existing rect guards hold
  // every fit and push until the box is trustworthy.
  const placed = new Set<string>();
  const paneBodyCbs = new Map<string, (el: HTMLDivElement | null) => void>();
  const termWrapCbs = new Map<string, (el: HTMLDivElement | null) => void>();

  const sync = () => {
    if (!container || !root) return;
    const cRect = container.getBoundingClientRect();
    for (const [paneId, bodyEl] of paneBodies) {
      const leaf = findLeaf(root, paneId);
      if (!leaf?.sessionId) continue;
      const wrap = termWraps.get(leaf.sessionId);
      if (!wrap) continue;
      const r = bodyEl.getBoundingClientRect();
      // A zero-rect body is itself mid-layout — placing the wrapper on
      // it would just trade one untrustworthy box for another.
      if (r.width <= 0 || r.height <= 0) continue;
      wrap.style.left = `${r.left - cRect.left}px`;
      wrap.style.top = `${r.top - cRect.top}px`;
      wrap.style.width = `${r.width}px`;
      wrap.style.height = `${r.height}px`;
      if (!placed.has(leaf.sessionId)) {
        placed.add(leaf.sessionId);
        logResizeGate(
          `pane-placed session=${leaf.sessionId} ` +
            `${Math.round(r.width)}x${Math.round(r.height)}`,
        );
      }
    }
  };

  return {
    sync,
    containerRef(el: HTMLDivElement | null) {
      container = el;
    },
    setRoot(next: PaneNode) {
      root = next;
    },
    paneBodyRefFor(paneId: string) {
      let cb = paneBodyCbs.get(paneId);
      if (!cb) {
        cb = (el) => {
          const prev = paneBodies.get(paneId);
          if (prev) ro?.unobserve(prev);
          if (el) {
            paneBodies.set(paneId, el);
            ro ??= new ResizeObserver(sync);
            ro.observe(el);
          } else {
            paneBodies.delete(paneId);
          }
        };
        paneBodyCbs.set(paneId, cb);
      }
      return cb;
    },
    termWrapRefFor(sessionId: string) {
      let cb = termWrapCbs.get(sessionId);
      if (!cb) {
        cb = (el) => {
          if (el) {
            termWraps.set(sessionId, el);
            // Hold the wrapper at 0×0 until sync() places it from a real
            // pane-body rect — see `placed`.
            if (!placed.has(sessionId)) {
              el.style.width = "0px";
              el.style.height = "0px";
            }
            // A wrapper can mount a commit AFTER its pane body has already
            // settled: restored-split hydration attaches the session
            // (adding it to `chats`) once the pane tree is stable, so
            // `layout.root` doesn't change and neither the geometry
            // layoutEffect nor the pane-body ResizeObserver fires. Position
            // it now from the existing pane rects. On the component's first
            // mount `root`/`container` aren't set yet so this no-ops,
            // leaving the layoutEffect to drive the initial sync.
            sync();
          } else {
            termWraps.delete(sessionId);
            placed.delete(sessionId);
          }
        };
        termWrapCbs.set(sessionId, cb);
      }
      return cb;
    },
    dispose() {
      ro?.disconnect();
    },
  };
}
