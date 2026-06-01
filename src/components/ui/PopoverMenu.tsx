import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

// Portal-positioned popover for dropdown menus. Renders the menu into
// <body> with fixed positioning derived from the anchor's rect, so it
// can't be clipped by an ancestor's overflow — Modal/Drawer scroll
// bodies, the Settings panel, etc. Handles outside-click (anchor OR
// menu), Escape, and repositioning on scroll/resize. The menu matches
// the anchor's width (at least `minWidth`) and is clamped to stay
// on-screen, flipping above the anchor when there's no room below.
const GAP = 4;
const EDGE = 8;
const EST_HEIGHT = 280;

export function PopoverMenu({
  open,
  anchorRef,
  onClose,
  minWidth = 0,
  children,
}: {
  open: boolean;
  anchorRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  minWidth?: number;
  children: ReactNode;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{
    top: number;
    left: number;
    width: number;
    flip: boolean;
  } | null>(null);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const update = () => {
      const anchor = anchorRef.current;
      if (!anchor) return;
      const r = anchor.getBoundingClientRect();
      const width = Math.max(r.width, minWidth);
      let left = r.left;
      if (left + width > window.innerWidth - EDGE) {
        left = window.innerWidth - EDGE - width;
      }
      if (left < EDGE) left = EDGE;
      const spaceBelow = window.innerHeight - r.bottom;
      const flip = spaceBelow < EST_HEIGHT && r.top > spaceBelow;
      setPos({ top: flip ? r.top - GAP : r.bottom + GAP, left, width, flip });
    };
    update();
    // Capture phase so the Modal/Drawer body's own scroll triggers a
    // reposition, keeping the menu attached to its anchor.
    window.addEventListener("scroll", update, true);
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("resize", update);
    };
  }, [open, anchorRef, minWidth]);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (anchorRef.current?.contains(t) || menuRef.current?.contains(t)) {
        return;
      }
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDoc, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDoc, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, anchorRef, onClose]);

  // Stop mousedown inside the (portaled) menu from bubbling to document
  // /window listeners that implement their own outside-click close —
  // e.g. SettingsModal closes on any document mousedown outside its
  // card, and the portaled menu lives outside that card in the DOM, so
  // selecting an option would otherwise close the modal mid-click.
  useEffect(() => {
    const node = menuRef.current;
    if (!open || !pos || !node) return;
    const stop = (e: Event) => e.stopPropagation();
    node.addEventListener("mousedown", stop);
    return () => node.removeEventListener("mousedown", stop);
  }, [open, pos]);

  if (!open || !pos) return null;
  return createPortal(
    <div
      ref={menuRef}
      style={{
        top: pos.top,
        left: pos.left,
        width: pos.width,
        transform: pos.flip ? "translateY(-100%)" : undefined,
      }}
      className="fixed z-[60]"
    >
      {children}
    </div>,
    document.body,
  );
}
