import type { InputHTMLAttributes, ReactNode, TextareaHTMLAttributes } from "react";

import { Info } from "lucide-react";

import { Tooltip } from "./Tooltip";

export function Label({
  htmlFor,
  children,
  hint,
}: {
  htmlFor: string;
  children: ReactNode;
  hint?: ReactNode;
}) {
  // Hints render as an info icon with a hover/focus tooltip rather than
  // inline text: long hints used to wrap and collide with the label.
  const hintLabel = typeof hint === "string" ? hint : undefined;
  return (
    <label
      htmlFor={htmlFor}
      className="flex items-center gap-1.5 text-xs font-medium text-fg-2"
    >
      <span>{children}</span>
      {hint ? (
        <Tooltip content={hint}>
          <span className="inline-flex text-fg-3" aria-label={hintLabel}>
            <Info className="h-3.5 w-3.5" aria-hidden />
          </span>
        </Tooltip>
      ) : null}
    </label>
  );
}

const inputBase =
  "w-full rounded border border-line-strong bg-bg px-2.5 py-1.5 text-sm text-fg placeholder:text-fg-3 focus:outline-none focus:border-fg-3 disabled:opacity-60";

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  const { className = "", ...rest } = props;
  return <input className={`${inputBase} ${className}`} {...rest} />;
}

export function Textarea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  const { className = "", ...rest } = props;
  return <textarea className={`${inputBase} font-mono ${className}`} {...rest} />;
}

export function FieldError({ children }: { children?: ReactNode }) {
  if (!children) return null;
  return <p className="text-xs text-danger">{children}</p>;
}

export function Field({
  id,
  label,
  hint,
  error,
  children,
}: {
  id: string;
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Label htmlFor={id} hint={hint}>
        {label}
      </Label>
      {children}
      <FieldError>{error}</FieldError>
    </div>
  );
}
