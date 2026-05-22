// Drag-to-resize state for a panel with persisted width.
//
// One small primitive shared by the left sidebar and the right side
// panels (RunnerChat, MissionWorkspace) so the three resize handles
// don't drift in clamps or persistence shape. The `edge` arg picks
// whether dragging right grows the panel (left sidebar, `right`) or
// dragging left grows it (right-side panels, `left`).
//
// Drag write path: while the user is dragging, we write directly to
// the target elements' `style.width` (via the refs passed in
// `targets`) and skip React renders entirely. State + localStorage
// get a single commit at the end of the drag. Two reasons:
//   - the panels these targets render contain heavy children (system
//     prompt readout, runners rail, xterm pane), and a setState per
//     pointermove re-renders the whole subtree → visible drag jank.
//   - the panels carry a `transition-[width]` class for the collapse
//     animation; if we leave the transition on during drag, each
//     style.width update fires a fresh 150–200ms transition and the
//     width visibly trails the cursor. We snapshot + clear inline
//     `transition` on every target for the drag, restore at end.
//
// Cursor lock + lost-release safety: this runs on Tauri desktop where
// the user can alt-tab mid-drag, release the mouse outside the OS
// window, or have the pointer otherwise yanked away. We use Pointer
// Events with `setPointerCapture` so the browser guarantees the drag
// terminates via `pointerup`, `pointercancel`, or
// `lostpointercapture` — all routed through one idempotent cleanup.
// A `blur` listener on window is added as belt-and-suspenders for
// the edge case where capture is somehow held across a window blur.
// Without these, a lost release would strand the fullscreen overlay
// and freeze the app until reload.
//
// The fullscreen transparent overlay during drag is what actually
// keeps the cursor consistent: `document.body.style.cursor` is
// inherited, but element-level cursor declarations (xterm uses
// `cursor: text` on its renderer) win as the pointer crosses them.
// The overlay sits at the top of the stacking context with
// `cursor: col-resize` and absorbs accidental clicks underneath.

import { useState, type RefObject } from "react";

export type ResizeEdge = "left" | "right";

export interface UseResizableWidthOptions {
  storageKey: string;
  defaultWidth: number;
  min: number;
  max: number;
  edge: ResizeEdge;
  /** Refs to elements whose `style.width` should track the cursor
   *  during drag. The hook also suspends each target's CSS
   *  transition for the duration of the drag so width updates render
   *  immediately instead of animating along the collapse curve. */
  targets: ReadonlyArray<RefObject<HTMLElement | null>>;
}

export interface UseResizableWidth {
  width: number;
  onResizeStart: (e: React.PointerEvent) => void;
}

export function useResizableWidth({
  storageKey,
  defaultWidth,
  min,
  max,
  edge,
  targets,
}: UseResizableWidthOptions): UseResizableWidth {
  const [width, setWidth] = useState<number>(() => {
    if (typeof localStorage === "undefined") return defaultWidth;
    const stored = localStorage.getItem(storageKey);
    if (stored) {
      const n = parseInt(stored, 10);
      if (!Number.isNaN(n) && n >= min && n <= max) return n;
    }
    return defaultWidth;
  });

  const onResizeStart = (e: React.PointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    const pointerId = e.pointerId;
    const startX = e.clientX;
    const startWidth = width;
    const sign = edge === "right" ? 1 : -1;
    let liveWidth = startWidth;

    const snapshot = targets
      .map((ref) => {
        const el = ref.current;
        if (!el) return null;
        const prevTransition = el.style.transition;
        el.style.transition = "none";
        return { el, prevTransition };
      })
      .filter(
        (x): x is { el: HTMLElement; prevTransition: string } => x !== null,
      );

    const overlay = document.createElement("div");
    overlay.style.cssText =
      "position:fixed;inset:0;z-index:2147483647;cursor:col-resize;user-select:none;";
    document.body.appendChild(overlay);
    const prevBodyUserSelect = document.body.style.userSelect;
    document.body.style.userSelect = "none";

    // Route capture so events still fire on `handle` even when the
    // pointer leaves the 4px strip. Some browsers/scenarios throw
    // (capture already held, pointer no longer active) — best-effort.
    try {
      handle.setPointerCapture(pointerId);
    } catch {
      // ignore — listeners below still terminate the drag via
      // pointerup/cancel even without capture.
    }

    let cleanedUp = false;
    const cleanup = () => {
      if (cleanedUp) return;
      cleanedUp = true;
      handle.removeEventListener("pointermove", onPointerMove);
      handle.removeEventListener("pointerup", onPointerEnd);
      handle.removeEventListener("pointercancel", onPointerEnd);
      handle.removeEventListener("lostpointercapture", onPointerEnd);
      window.removeEventListener("blur", onPointerEnd);
      try {
        handle.releasePointerCapture(pointerId);
      } catch {
        // already released / never captured — fine.
      }
      overlay.remove();
      document.body.style.userSelect = prevBodyUserSelect;
      for (const { el, prevTransition } of snapshot) {
        el.style.transition = prevTransition;
      }
      setWidth(liveWidth);
      try {
        localStorage.setItem(storageKey, String(liveWidth));
      } catch {
        // ignore quota / disabled-storage errors
      }
    };

    const onPointerMove = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      const next = Math.min(
        max,
        Math.max(min, startWidth + sign * (ev.clientX - startX)),
      );
      liveWidth = next;
      for (const { el } of snapshot) {
        el.style.width = `${next}px`;
      }
    };

    const onPointerEnd = () => {
      cleanup();
    };

    handle.addEventListener("pointermove", onPointerMove);
    handle.addEventListener("pointerup", onPointerEnd);
    handle.addEventListener("pointercancel", onPointerEnd);
    handle.addEventListener("lostpointercapture", onPointerEnd);
    window.addEventListener("blur", onPointerEnd);
  };

  return { width, onResizeStart };
}
