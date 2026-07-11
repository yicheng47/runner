// Themed dropdown. The native `<select>` renders the platform's
// chrome-gradient control on macOS regardless of CSS, which clashes
// with the dark theme — same reason `RuntimeSelect` exists. This is
// a generic value/label variant of that pattern, shared by the
// settings panes and the runner-edit forms (e.g. the Permission mode
// picker).

import { useRef, useState } from "react";

import { PopoverMenu } from "./PopoverMenu";

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
  /// Optional hex color rendered as a 12×12 rounded square to the
  /// left of the label, in both the trigger and the listbox row.
  /// Used by the Appearance pane theme dropdowns so the user previews
  /// the accent before committing.
  swatchColor?: string;
}

export function StyledSelect({
  id,
  value,
  options,
  onChange,
  className,
  buttonLabel,
  disabled,
}: {
  id?: string;
  value: string;
  options: StyledSelectOption[];
  onChange: (v: string) => void;
  /// Override the wrapper's min-width / width when the caller wants a
  /// wider trigger (e.g. the Permission mode dropdown). Defaults to
  /// the existing `min-w-[160px]` shape the settings panes ship with.
  className?: string;
  /// Override the trigger's displayed label. Defaults to the matching
  /// option's `label`. Used when the caller wants a fixed prefix
  /// (e.g. "Permission: …") in the trigger but plain labels in the
  /// listbox.
  buttonLabel?: string;
  /// When true, the trigger stops being interactive (no listbox open,
  /// muted chrome). Used by the Appearance pane for the Dark theme
  /// dropdown — v1 ships only Carbon, so the row is informational.
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const current = options.find((o) => o.value === value) ?? options[0];
  const triggerLabel = buttonLabel ?? current?.label ?? "";

  return (
    <div ref={rootRef} className={className ?? "min-w-[160px]"}>
      <button
        id={id}
        type="button"
        onClick={() => {
          if (disabled) return;
          setOpen((v) => !v);
        }}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-disabled={disabled}
        className={`flex w-full items-center justify-between gap-2 rounded border border-line-strong bg-bg px-2.5 py-1.5 text-left text-sm text-fg transition-colors focus:outline-none ${
          disabled
            ? "cursor-not-allowed opacity-60"
            : "cursor-pointer hover:border-fg-3 focus:border-fg-3"
        }`}
      >
        <span className="flex min-w-0 items-center gap-2">
          {current?.swatchColor ? (
            <span
              aria-hidden
              className="h-3 w-3 shrink-0 rounded-sm"
              style={{ backgroundColor: current.swatchColor }}
            />
          ) : null}
          <span className="truncate">{triggerLabel}</span>
        </span>
        <span
          aria-hidden
          className={`text-fg-3 transition-transform ${open ? "rotate-180" : ""}`}
        >
          ▾
        </span>
      </button>
      <PopoverMenu
        open={open}
        anchorRef={rootRef}
        onClose={() => setOpen(false)}
        minWidth={240}
      >
        <ul
          role="listbox"
          className="flex max-h-[260px] w-full flex-col overflow-y-auto rounded border border-line-strong bg-panel py-1 shadow-xl"
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
                  className={`flex w-full cursor-pointer flex-col items-start gap-0.5 px-3 py-2 text-left text-sm transition-colors ${tone}`}
                >
                  <span className="flex w-full items-center justify-between gap-2">
                    <span className="flex min-w-0 items-center gap-2">
                      {opt.swatchColor ? (
                        <span
                          aria-hidden
                          className="h-3 w-3 shrink-0 rounded-sm"
                          style={{ backgroundColor: opt.swatchColor }}
                        />
                      ) : null}
                      <span className="truncate font-medium">{opt.label}</span>
                    </span>
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
      </PopoverMenu>
    </div>
  );
}
