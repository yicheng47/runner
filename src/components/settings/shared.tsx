// Shared shells for the full-page settings panes — the card + hairline
// row pattern from `design/runner-setting.pen`. Cards are `--color-panel`
// surfaces on the page's `--color-bg`; rows divide with the theme's
// hairline token so light themes work by construction.

import { Minus, Plus } from "lucide-react";

export function PaneHeader({
  title,
  subtitle,
  action,
}: {
  title: string;
  subtitle: string;
  /** Right-aligned control on the title row (design: Archived's
   *  Delete all sits beside the page title, not in the toolbar). */
  action?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-6">
        <h2 className="text-[20px] font-semibold text-fg">{title}</h2>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      <p className="text-[13px] text-fg-2">{subtitle}</p>
    </div>
  );
}

export function SettingsCard({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-col divide-y divide-line overflow-hidden rounded-xl border border-line bg-panel">
      {children}
    </div>
  );
}

export function SettingsRow({
  label,
  sub,
  children,
}: {
  label: string;
  sub?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-fg">{label}</span>
        {sub ? <span className="text-[11px] text-fg-2">{sub}</span> : null}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

// Generic [−] <value> [+] stepper. Caller renders the value cell's
// contents and supplies its width.
export function Stepper({
  valueCellWidth,
  decDisabled,
  incDisabled,
  onDec,
  onInc,
  decAriaLabel,
  incAriaLabel,
  children,
}: {
  valueCellWidth: number;
  decDisabled?: boolean;
  incDisabled?: boolean;
  onDec: () => void;
  onInc: () => void;
  decAriaLabel: string;
  incAriaLabel: string;
  children: React.ReactNode;
}) {
  const buttonClass =
    "flex h-[30px] w-[30px] shrink-0 cursor-pointer items-center justify-center text-fg-3 transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:text-fg-3";
  return (
    <div className="flex h-[30px] items-center rounded-md border border-line bg-bg">
      <button
        type="button"
        onClick={onDec}
        disabled={decDisabled}
        aria-label={decAriaLabel}
        className={buttonClass}
      >
        <Minus aria-hidden className="h-3.5 w-3.5" />
      </button>
      <div
        style={{ width: valueCellWidth }}
        className="flex h-[30px] items-center justify-center border-x border-line"
      >
        {children}
      </div>
      <button
        type="button"
        onClick={onInc}
        disabled={incDisabled}
        aria-label={incAriaLabel}
        className={buttonClass}
      >
        <Plus aria-hidden className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
