import { useRef, useState } from "react";

import { useT } from "../../lib/i18n";
import { PopoverMenu } from "./PopoverMenu";
import { modelSuggestions } from "./runtimes";

// Model picker shared by the create modal and edit drawer. An editable
// combobox: the field is free text (type any model name; empty means
// the runtime's own default), and the ▾ reveals curated alias shortcuts
// for the runtime. The backend takes the value verbatim as
// `--model <name>`, so there's no closed set and no separate "custom"
// mode — picking a suggestion just fills the input.
//
// The suggestion list is a PopoverMenu (portaled, like StyledSelect) so
// it can't be clipped by the modal/drawer scroll body.
export function ModelField({
  id,
  runtime,
  model,
  onModelChange,
}: {
  id: string;
  runtime: string;
  model: string;
  onModelChange: (model: string) => void;
}) {
  const t = useT();
  const suggestions = modelSuggestions(runtime);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const hasSuggestions = suggestions.length > 0;

  return (
    <div ref={rootRef} className="relative">
      <input
        id={id}
        type="text"
        value={model}
        placeholder={t("default")}
        onChange={(e) => onModelChange(e.target.value)}
        // Clicking the field toggles the suggestion list — so clicking
        // it again closes rather than re-opening (matches the other
        // selectors' trigger behaviour). The field stays freely
        // editable; Escape / click-outside / picking an option close it.
        onMouseDown={() => hasSuggestions && setOpen((v) => !v)}
        // Styled to match the form's other selectors (RuntimeSelect /
        // StyledSelect) so the editable combobox reads as the same
        // family of control.
        className={`w-full rounded border border-line-strong bg-bg px-2.5 py-1.5 text-sm text-fg transition-colors placeholder:text-fg-3 hover:border-fg-3 focus:border-fg-3 focus:outline-none ${
          hasSuggestions ? "pr-8" : ""
        }`}
      />
      {hasSuggestions ? (
        <button
          type="button"
          aria-label={t("Choose a model")}
          aria-haspopup="listbox"
          aria-expanded={open}
          tabIndex={-1}
          onClick={() => setOpen((v) => !v)}
          className="absolute inset-y-0 right-0 flex items-center px-2.5 text-fg-3 transition-colors hover:text-fg-2"
        >
          <span
            aria-hidden
            className={`transition-transform ${open ? "rotate-180" : ""}`}
          >
            ▾
          </span>
        </button>
      ) : null}
      <PopoverMenu
        open={open && hasSuggestions}
        anchorRef={rootRef}
        onClose={() => setOpen(false)}
      >
        <ul
          role="listbox"
          className="flex max-h-[260px] w-full flex-col overflow-y-auto rounded border border-line-strong bg-panel py-1 shadow-xl"
        >
          {suggestions.map((opt) => {
            const active = opt.value === model;
            return (
              <li key={opt.value || "default"} role="option" aria-selected={active}>
                <button
                  type="button"
                  onClick={() => {
                    onModelChange(opt.value);
                    setOpen(false);
                  }}
                  className={`flex w-full cursor-pointer flex-col items-start gap-0.5 px-3 py-2 text-left text-sm transition-colors ${
                    active ? "bg-raised text-fg" : "text-fg-2 hover:bg-raised"
                  }`}
                >
                  <span className="flex w-full items-center justify-between gap-2">
                    <span className="truncate font-medium">{opt.label}</span>
                    {active ? (
                      <span aria-hidden className="text-accent">
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
