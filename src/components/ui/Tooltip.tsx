import { useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

// Lightweight hover/focus tooltip. Portals to <body> with fixed
// positioning computed from the trigger's rect so it can't be clipped
// by the overflow on Modal (`overflow-hidden`) / Drawer
// (`overflow-y-auto`) containers. No dependency, no positioning lib —
// the content is short label-hint text, so a simple centered-above
// placement is enough.
export function Tooltip({
  content,
  children,
}: {
  content: ReactNode;
  children: ReactNode;
}) {
  const triggerRef = useRef<HTMLSpanElement>(null);
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(
    null,
  );

  const show = () => {
    const el = triggerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setCoords({ top: rect.top, left: rect.left + rect.width / 2 });
  };
  const hide = () => setCoords(null);

  return (
    <span
      ref={triggerRef}
      className="inline-flex"
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
    >
      {children}
      {content && coords
        ? createPortal(
            <span
              role="tooltip"
              style={{
                top: coords.top - 6,
                left: coords.left,
                transform: "translate(-50%, -100%)",
              }}
              className="pointer-events-none fixed z-[60] block w-max max-w-xs rounded border border-line-strong bg-raised px-2 py-1 text-[11px] font-normal leading-snug text-fg-2 shadow-lg"
            >
              {content}
            </span>,
            document.body,
          )
        : null}
    </span>
  );
}
