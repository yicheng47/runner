// Custom dark-themed runtime picker. Replaces the native `<select>` so
// the dropdown surface matches the Carbon & Plasma theme on macOS (the
// system control renders as a chrome-gradient button regardless of
// CSS).

import { useRef, useState } from "react";

import { PopoverMenu } from "./PopoverMenu";
import { RUNTIME_OPTIONS, type RuntimeOption } from "./runtimes";

export function RuntimeSelect({
  id,
  value,
  onChange,
  options = RUNTIME_OPTIONS,
}: {
  id?: string;
  value: string;
  onChange: (option: RuntimeOption) => void;
  options?: RuntimeOption[];
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const current = options.find((o) => o.value === value) ?? options[0];

  return (
    <div ref={rootRef}>
      <button
        id={id}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center justify-between rounded border border-line-strong bg-bg px-2.5 py-1.5 text-left text-sm text-fg transition-colors hover:border-fg-3 focus:border-fg-3 focus:outline-none"
      >
        <span className="font-mono">{current.label}</span>
        <span
          aria-hidden
          className={`ml-2 text-fg-3 transition-transform ${open ? "rotate-180" : ""}`}
        >
          ▾
        </span>
      </button>

      <PopoverMenu open={open} anchorRef={rootRef} onClose={() => setOpen(false)}>
        <ul
          role="listbox"
          className="flex w-full max-h-[260px] flex-col overflow-y-auto rounded border border-line-strong bg-panel py-1 shadow-xl"
        >
          {options.map((opt) => {
            const active = opt.value === value;
            return (
              <li key={opt.value} role="option" aria-selected={active}>
                <button
                  type="button"
                  onClick={() => {
                    onChange(opt);
                    setOpen(false);
                  }}
                  className={`flex w-full cursor-pointer flex-col items-start gap-0.5 px-3 py-2 text-left text-sm transition-colors hover:bg-raised ${
                    active ? "bg-raised text-fg" : "text-fg-2"
                  }`}
                >
                  <span className="flex w-full items-center justify-between gap-2 font-mono text-fg">
                    <span>{opt.label}</span>
                    {active ? (
                      <span className="text-accent" aria-hidden>
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
