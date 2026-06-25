import { useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";

interface CopyValueButtonProps {
  value: string | null | undefined;
  label: string;
}

export function CopyValueButton({ value, label }: CopyValueButtonProps) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    };
  }, []);

  if (!value) return null;

  const copyValue = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.error("CopyValueButton: clipboard write failed", e);
    }
  };

  return (
    <button
      type="button"
      aria-label={label}
      title={copied ? "Copied" : label}
      onClick={(e) => {
        e.stopPropagation();
        void copyValue();
      }}
      onKeyDown={(e) => e.stopPropagation()}
      className="inline-flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-line/60 hover:text-fg focus:bg-line/60 focus:text-fg focus:outline-none"
    >
      {copied ? (
        <Check aria-hidden className="h-3 w-3" />
      ) : (
        <Copy aria-hidden className="h-3 w-3" />
      )}
    </button>
  );
}
