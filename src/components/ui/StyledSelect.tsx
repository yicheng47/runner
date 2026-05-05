// Themed dropdown. The native `<select>` renders the platform's
// chrome-gradient control on macOS regardless of CSS, which clashes
// with the dark theme — same reason `RuntimeSelect` exists. This is
// a generic value/label variant of that pattern, lifted out of
// `SettingsModal.tsx` so the runner-edit forms can reuse it for the
// Permission mode picker.

import { useEffect, useRef, useState } from "react";

export interface StyledSelectOption {
  value: string;
  label: string;
  /// Optional helper line shown below the label inside the listbox.
  /// Gives the user the per-option meaning without crowding the
  /// trigger button.
  description?: string;
  /// Marks an option that's the destructive choice (e.g. Bypass) so
  /// the listbox renders it with the danger palette.
  danger?: boolean;
}

export function StyledSelect({
  value,
  options,
  onChange,
  className,
  buttonLabel,
}: {
  value: string;
  options: StyledSelectOption[];
  onChange: (v: string) => void;
  /// Override the wrapper's min-width / width when the caller wants a
  /// wider trigger (e.g. the Permission mode dropdown). Defaults to
  /// the existing `min-w-[160px]` shape `SettingsModal` shipped with.
  className?: string;
  /// Override the trigger's displayed label. Defaults to the matching
  /// option's `label`. Used when the caller wants a fixed prefix
  /// (e.g. "Permission: …") in the trigger but plain labels in the
  /// listbox.
  buttonLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDoc);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDoc);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = options.find((o) => o.value === value) ?? options[0];
  const triggerLabel = buttonLabel ?? current?.label ?? "";

  return (
    <div ref={rootRef} className={`relative ${className ?? "min-w-[160px]"}`}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center justify-between gap-2 rounded-md border border-line bg-bg px-3 py-2 text-left text-[12px] text-fg transition-colors hover:border-line-strong focus:border-fg-3 focus:outline-none"
      >
        <span className="truncate">{triggerLabel}</span>
        <span
          aria-hidden
          className={`text-fg-3 transition-transform ${open ? "rotate-180" : ""}`}
        >
          ▾
        </span>
      </button>
      {open ? (
        <ul
          role="listbox"
          className="absolute right-0 top-full z-30 mt-1 flex max-h-[260px] w-[280px] flex-col overflow-y-auto rounded-md border border-line bg-panel py-1 shadow-[0_8px_24px_rgba(0,0,0,0.5)]"
        >
          {options.map((opt) => {
            const active = opt.value === value;
            const tone = active
              ? opt.danger
                ? "bg-danger/10 text-danger"
                : "bg-raised text-fg"
              : opt.danger
                ? "text-danger/80 hover:bg-danger/10"
                : "text-fg-2 hover:bg-raised";
            return (
              <li
                key={opt.value || "__none__"}
                role="option"
                aria-selected={active}
              >
                <button
                  type="button"
                  onClick={() => {
                    onChange(opt.value);
                    setOpen(false);
                  }}
                  className={`flex w-full cursor-pointer flex-col items-start gap-0.5 px-3 py-2 text-left text-[12px] transition-colors ${tone}`}
                >
                  <span className="flex w-full items-center justify-between gap-2">
                    <span className="truncate font-medium">{opt.label}</span>
                    {active ? (
                      <span
                        aria-hidden
                        className={opt.danger ? "text-danger" : "text-accent"}
                      >
                        ✓
                      </span>
                    ) : null}
                  </span>
                  {opt.description ? (
                    <span className="text-[11px] text-fg-3">
                      {opt.description}
                    </span>
                  ) : null}
                </button>
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
