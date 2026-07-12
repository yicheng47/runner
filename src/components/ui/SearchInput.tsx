import { Search, X } from "lucide-react";

export function SearchInput({
  value,
  onChange,
  label,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  placeholder: string;
}) {
  return (
    <div className="flex w-full max-w-[320px] items-center gap-2 rounded border border-line bg-bg px-3 py-2 transition-colors focus-within:border-line-strong">
      <Search aria-hidden className="h-3.5 w-3.5 shrink-0 text-fg-3" />
      <input
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.preventDefault();
          event.stopPropagation();
          onChange("");
        }}
        aria-label={label}
        placeholder={placeholder}
        className="min-w-0 flex-1 bg-transparent text-[13px] text-fg outline-none placeholder:text-fg-3"
      />
      {value ? (
        <button
          type="button"
          onClick={() => onChange("")}
          aria-label={`Clear ${label.toLowerCase()}`}
          className="flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-raised hover:text-fg focus:outline-none focus-visible:ring-2 focus-visible:ring-line-strong"
        >
          <X aria-hidden className="h-3.5 w-3.5" />
        </button>
      ) : null}
      <span className="rounded border border-line bg-raised px-1.5 py-px font-mono text-[10px] leading-none text-fg-3">
        esc
      </span>
    </div>
  );
}
